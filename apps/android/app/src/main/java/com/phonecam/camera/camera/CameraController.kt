package com.phonecam.camera.camera

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Rect
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraConstrainedHighSpeedCaptureSession
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.params.ColorSpaceTransform
import android.hardware.camera2.params.OutputConfiguration
import android.hardware.camera2.params.RggbChannelVector
import android.hardware.camera2.params.SessionConfiguration
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.util.Log
import android.util.Range
import android.util.Size
import android.view.Surface
import android.graphics.SurfaceTexture
import java.util.concurrent.ConcurrentHashMap

/** Physical lens classification for rear multi-camera systems. */
enum class LensType(val displayName: String, val defaultZoom: Float) {
    ULTRA_WIDE("Ultra-Wide", 0.5f),
    WIDE("Main Wide", 1.0f),
    TELEPHOTO("Telephoto", 2.0f),
    FRONT("Front", 1.0f);
}

/** Describes an individual physical lens identified on the device. */
data class PhysicalLens(
    val id: String,
    val type: LensType,
    val focalLengthMm: Float?,
    val focalLength35mmEquiv: Float?,
    val zoomFactor: Float = type.defaultZoom,
    val isFront: Boolean = type == LensType.FRONT,
)

/** Describes RGB channel gains for manual white balance. */
data class ColorGains(
    val red: Float,
    val greenEven: Float,
    val greenOdd: Float,
    val blue: Float,
) {
    fun toRggbVector(): RggbChannelVector = RggbChannelVector(red, greenEven, greenOdd, blue)
}

/**
 * Camera2 capture into a Surface.
 *
 * Camera2 rather than CameraX because the encoder needs its own input Surface as
 * a session target. CameraX's `VideoCapture` drives its own `Recorder` and does
 * not expose a raw encoder Surface, and routing frames through an intermediate
 * `SurfaceTexture` would add a GPU copy to every frame on the latency-critical
 * path (PRD §46, §47).
 *
 * Capability-based configuration is preserved: the requested size and frame rate
 * are matched against what this camera actually advertises, and a substitution is
 * reported rather than failing (PRD §13, Rule 5, Rule 6).
 */
class CameraController(private val context: Context) {

    private val cameraManager = context.getSystemService(Context.CAMERA_SERVICE) as CameraManager
    private val characteristicsCache = ConcurrentHashMap<String, CameraCharacteristics>()

    init {
        runCatching {
            for (id in cameraManager.cameraIdList) {
                runCatching { cameraManager.getCameraCharacteristics(id) }
                    .getOrNull()
                    ?.let { characteristicsCache[id] = it }
            }
        }
    }

    private fun getCharacteristics(cameraId: String): CameraCharacteristics? {
        characteristicsCache[cameraId]?.let { return it }
        var delayMs = 50L
        for (attempt in 0 until 6) {
            val result = runCatching { cameraManager.getCameraCharacteristics(cameraId) }
                .onFailure { Log.w(TAG, "camera $cameraId characteristics attempt $attempt failed", it) }
                .getOrNull()
            if (result != null) {
                characteristicsCache[cameraId] = result
                return result
            }
            SystemClock.sleep(delayMs)
            delayMs = (delayMs * 2).coerceAtMost(250L)
        }
        return null
    }

    private var thread: HandlerThread? = null
    private var handler: Handler? = null
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var requestBuilder: CaptureRequest.Builder? = null
    @Volatile
    private var cameraGeneration = 0L

    /**
     * The on-screen viewfinder surface, if the UI has one attached.
     *
     * Held here rather than passed once, so that a viewfinder appearing or
     * disappearing while a stream is live is handled by rebuilding the capture
     * session instead of restarting the stream.
     */
    @Volatile
    private var previewSurface: Surface? = null

    /** The live session's parameters, kept so it can be rebuilt for a new output. */
    private var sessionRequest: SessionRequest? = null

    /**
     * What is needed to build the capture session.
     *
     * Retained because the set of outputs can change while the camera stays open:
     * the user scrolling the viewfinder into view must not cost them the stream.
     */
    private data class SessionRequest(
        val targetSurface: Surface,
        val targetSize: Size,
        val resolved: ResolvedVideoConfig,
        val onError: (String) -> Unit,
    )

    @Volatile
    private var torchEnabled = false
    @Volatile
    private var zoom = 1f
    @Volatile
    private var exposureCompensation = 0
    @Volatile
    private var whiteBalanceKelvin: Int? = null
    @Volatile
    private var manualFocusDistance: Float? = null
    @Volatile
    private var lastCameraId: String? = null

    val currentExposure: Int
        get() = exposureCompensation

    val currentWhiteBalance: Int?
        get() = whiteBalanceKelvin

    val currentFocusDistance: Float?
        get() = manualFocusDistance

    @Volatile
    private var activeConfig: ResolvedVideoConfig? = null

    /** The size the camera is really producing, which may differ from the request. */
    val resolvedConfig: ResolvedVideoConfig?
        get() = activeConfig

    /**
     * Sizes this camera can capture at >= 60 fps in a constrained high-speed session.
     */
    fun highSpeedSizes(cameraId: String): List<Pair<Int, Int>> {
        val map = getCharacteristics(cameraId)
            ?.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return emptyList()
        val sizes = map.highSpeedVideoSizes ?: return emptyList()
        return sizes
            .filter { size ->
                runCatching { map.getHighSpeedVideoFpsRangesFor(size)?.any { it.upper >= 60 } == true }
                    .getOrDefault(false)
            }
            .map { it.width to it.height }
    }

    /** True when normal AE ranges support 60 fps (e.g. `[60, 60]` or `[30, 60]`). */
    fun supportsNormalAe60(cameraId: String): Boolean {
        val ranges = getCharacteristics(cameraId)
            ?.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
            ?: return false
        return ranges.any { it.upper >= 60 }
    }

    /**
     * Frame rates this camera genuinely supports (via normal AE ranges or high-speed configs).
     */
    fun supportedFrameRates(cameraId: String): List<Int> {
        val characteristics = getCharacteristics(cameraId) ?: return emptyList()
        val ranges = characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
        val out = linkedSetOf<Int>()
        ranges?.forEach { range ->
            if (range.upper in 1..120) out += range.upper
            if (range.lower in 1..120) out += range.lower
        }
        val normal = listOf(24, 25, 30).filter { candidate -> out.any { it >= candidate } }
        val supports60 = out.any { it >= 60 } || supportsHighSpeedFps(characteristics, 60)
        return (normal + if (supports60) listOf(60) else emptyList())
            .distinct()
            .ifEmpty { listOf(30) }
    }

    /** True when some high speed configuration reaches [fps]. */
    private fun supportsHighSpeedFps(characteristics: CameraCharacteristics, fps: Int): Boolean {
        val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return false
        val sizes = map.highSpeedVideoSizes ?: return false
        return sizes.any { size ->
            map.getHighSpeedVideoFpsRangesFor(size)?.any { it.upper >= fps } == true
        }
    }
    /** Output sizes for encoder surfaces, with fallbacks across MediaRecorder,
     * generic Surface, and SurfaceTexture classes to capture actual supported sizes (including 4K). */
    private fun outputSizes(map: android.hardware.camera2.params.StreamConfigurationMap): Array<Size> {
        val recorderSizes = map.getOutputSizes(android.media.MediaRecorder::class.java) ?: emptyArray()
        val surfaceSizes = map.getOutputSizes(Surface::class.java) ?: emptyArray()
        val textureSizes = map.getOutputSizes(SurfaceTexture::class.java) ?: emptyArray()
        val all = (recorderSizes + surfaceSizes + textureSizes).distinctBy { it.width to it.height }
        return if (all.isNotEmpty()) all.toTypedArray() else (map.getOutputSizes(SurfaceTexture::class.java) ?: emptyArray())
    }

    /** Output sizes this camera can stream, largest first, landscape-normalised. */
    fun supportedSizes(cameraId: String): List<Pair<Int, Int>> {
        val characteristics = getCharacteristics(cameraId) ?: return emptyList()
        val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            ?: return emptyList()
        return outputSizes(map)
            .filter { it.width >= 320 && it.height >= 240 }
            .map { if (it.width >= it.height) it.width to it.height else it.height to it.width }
            .filter { (width, height) -> width <= MAX_VIDEO_WIDTH && height <= MAX_VIDEO_HEIGHT }
            .distinct()
            .sortedByDescending { it.first.toLong() * it.second }
    }


    /**
     * Opens [cameraId] and starts streaming into [targetSurface].
     *
     * Returns the configuration that was actually applied, including any
     * substitution the device forced.
     */
    @SuppressLint("MissingPermission")
    fun start(
        cameraId: String,
        targetSurface: Surface,
        requestedResolution: String?,
        requestedFps: Int?,
        requestedBitrate: Int?,
        onStarted: (ResolvedVideoConfig) -> Unit,
        onError: (String) -> Unit,
        /**
         * An optional second output the frames are mirrored to.
         *
         * This is the on-screen viewfinder. It is deliberately part of the same
         * capture session as the encoder rather than a second camera open: two
         * sessions on one camera would halve the frame rate and, on many devices,
         * refuse to open at all. Preview is also the reason a user can tell the
         * camera is working before a PC is involved.
         */
        previewSurface: Surface? = null,
        /**
         * Keep the camera handler thread alive across teardown and reconfiguration.
         */
        keepThread: Boolean = true,
    ) {
        if (keepThread && thread != null) {
            closeDevice()
        } else {
            stop()
        }
        if (lastCameraId != null && lastCameraId != cameraId) {
            torchEnabled = false
            zoom = 1f
        }
        lastCameraId = cameraId
        this.previewSurface = previewSurface

        val characteristics = getCharacteristics(cameraId) ?: run {
            onError("This camera isn't available on your phone.")
            return
        }

        val sizes = supportedSizes(cameraId)
        val fps = supportedFrameRates(cameraId)
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = requestedResolution,
            requestedFps = requestedFps,
            requestedBitrate = requestedBitrate,
            supportedSizes = sizes,
            supportedFps = fps,
            highSpeedSizes = highSpeedSizes(cameraId),
            supportsNormalAe60 = supportsNormalAe60(cameraId),
        )
        activeConfig = resolved

        val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
        val useHighSpeedSizeMatch = !supportsNormalAe60(cameraId) && resolved.fps >= 60
        val highSpeedExact = if (useHighSpeedSizeMatch) {
            map?.highSpeedVideoSizes?.firstOrNull { size ->
                size.width == resolved.width && size.height == resolved.height
            } ?: map?.highSpeedVideoSizes?.firstOrNull { size ->
                size.width == resolved.height && size.height == resolved.width
            }
        } else {
            null
        }
        val exact = highSpeedExact ?: map?.let { outputSizes(it) }?.firstOrNull { size ->
            (size.width == resolved.width && size.height == resolved.height) ||
                (size.width == resolved.height && size.height == resolved.width)
        }
        val targetSize = exact ?: Size(resolved.width, resolved.height)

        val generation = cameraGeneration
        startThread()

        val callback = object : CameraDevice.StateCallback() {
            override fun onOpened(camera: CameraDevice) {
                if (generation != cameraGeneration) {
                    camera.close()
                    return
                }
                device = camera
                createSession(
                    camera,
                    targetSurface,
                    targetSize,
                    resolved,
                    onStarted,
                    onError,
                    this@CameraController.previewSurface,
                    generation,
                )
            }

            override fun onDisconnected(camera: CameraDevice) {
                camera.close()
                if (generation != cameraGeneration) return
                device = null
                onError("The camera was disconnected. Unplugging the phone or another app taking the camera can cause this.")
            }

            override fun onError(camera: CameraDevice, error: Int) {
                camera.close()
                if (generation != cameraGeneration) return
                device = null
                if ((error == ERROR_CAMERA_IN_USE || error == ERROR_MAX_CAMERAS_IN_USE) && generation == cameraGeneration) {
                    // Under rapid camera switching or start/stop toggles, Camera2 HAL may still be
                    // completing the close of the previous camera instance. A brief retry avoids failing.
                    handler?.postDelayed({
                        if (generation == cameraGeneration && device == null) {
                            try {
                                cameraManager.openCamera(cameraId, this, handler)
                            } catch (_: Throwable) {
                                onError("Your camera is being used by another app. Close the other camera app and retry.")
                            }
                        }
                    }, 200L)
                    return
                }
                val message = when (error) {
                    ERROR_CAMERA_IN_USE, ERROR_MAX_CAMERAS_IN_USE ->
                        "Your camera is being used by another app. Close the other camera app and retry."
                    ERROR_CAMERA_DISABLED ->
                        "The camera is disabled by a device policy on this phone."
                    ERROR_CAMERA_DEVICE ->
                        "The camera reported a hardware error. Try restarting the phone."
                    else ->
                        "Couldn't open the camera. Close any other camera app and retry."
                }
                onError(message)
            }
        }

        try {
            cameraManager.openCamera(cameraId, callback, handler)
        } catch (t: Throwable) {
            Log.e(TAG, "openCamera failed", t)
            onError("Couldn't open the camera. Close any other camera app and retry.")
        }
    }

    /**
     * Chooses the optimal AE target FPS range for [targetFps].
     *
     * Prioritizes exact fixed ranges (e.g. [30, 30] or [60, 60]) so the sensor
     * maintains a rock-solid framerate without dropping to 15 FPS during motion
     * or indoor lighting. Falls back to variable ranges only if the sensor
     * does not support a fixed target rate.
     */
    internal fun selectFpsRange(characteristics: CameraCharacteristics, targetFps: Int): Range<Int> {
        val ranges = characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES) ?: emptyArray()
        val pairs = ranges.map { (it.lower ?: 0) to (it.upper ?: 0) }
        val chosen = selectFpsRangeFromPairs(pairs, targetFps)
        return Range(chosen.first, chosen.second)
    }

    /**
     * Frame rate range for a constrained high-speed session.
     *
     * Camera2 only accepts a range the high-speed configuration itself advertises
     * for [size]. Prefers `[60, 60]`, then `[30, 60]`, then `[30, 120]` / lowest
     * upper bound `>= 60` so devices whose AE ranges top out at 30 fps (e.g.
     * Snapdragon 680 exposing 1280x720 `[30, 120]` / `[120, 120]`) run reliably.
     */
    private fun selectHighSpeedRange(characteristics: CameraCharacteristics, size: Size): Range<Int> {
        val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
        val ranges = runCatching { map?.getHighSpeedVideoFpsRangesFor(size) }.getOrNull() ?: emptyArray()
        val pairs = ranges.map { (it.lower ?: 0) to (it.upper ?: 0) }
        val chosen = selectHighSpeedRangeFromPairs(pairs)
        return Range(chosen.first, chosen.second)
    }

    /**
     * Re-applies a modified request to the live session.
     *
     * A high speed session throws on `setRepeatingRequest`; it only takes a burst.
     * Torch and zoom are the two controls that rebuild the request mid-stream, so
     * they go through here rather than calling the session directly.
     */
    private fun applyRequest(builder: CaptureRequest.Builder) {
        val current = session ?: return
        runCatching {
            if (current is CameraConstrainedHighSpeedCaptureSession) {
                current.setRepeatingBurst(
                    current.createHighSpeedRequestList(builder.build()),
                    null,
                    handler,
                )
            } else {
                current.setRepeatingRequest(builder.build(), null, handler)
            }
        }.onFailure { Log.w(TAG, "applyRequest rejected by camera session: ${it.message}") }
    }

    @Volatile
    private var configuredAntibanding: String = "auto"

    /** Re-applies the requested antibanding mode without tearing down the session. */
    fun setAntibandingMode(mode: String) {
        configuredAntibanding = mode.lowercase()
        val builder = requestBuilder ?: return
        val characteristics = device?.id?.let { getCharacteristics(it) } ?: return
        val targetMode = resolveAntibandingMode(characteristics, configuredAntibanding)
        runCatching {
            builder.set(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE, targetMode)
            applyRequest(builder)
            Log.i(TAG, "Applied antibanding mode: $configuredAntibanding ($targetMode)")
        }.onFailure { Log.d(TAG, "antibanding change rejected: ${it.javaClass.simpleName}") }
    }

    /**
     * Resolves antibanding mode based on user preference and hardware support.
     * AUTO dynamically detects 50 Hz vs 60 Hz flicker, while 50 Hz and 60 Hz lock to AC mains frequency.
     */
    internal fun resolveAntibandingMode(characteristics: CameraCharacteristics, requested: String): Int {
        val modes = characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_ANTIBANDING_MODES) ?: intArrayOf()
        return when (requested.lowercase()) {
            "50hz", "50" -> if (modes.contains(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_50HZ)) {
                CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_50HZ
            } else if (modes.contains(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_AUTO)) {
                CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_AUTO
            } else CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_OFF

            "60hz", "60" -> if (modes.contains(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_60HZ)) {
                CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_60HZ
            } else if (modes.contains(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_AUTO)) {
                CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_AUTO
            } else CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_OFF

            "off" -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_OFF

            else -> when {
                modes.contains(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_AUTO) -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_AUTO
                modes.contains(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_50HZ) -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_50HZ
                modes.contains(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_60HZ) -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_60HZ
                else -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_OFF
            }
        }
    }


    /**
     * Safely selects autofocus mode based on camera capabilities.
     * Prioritizes CONTINUOUS_PICTURE over CONTINUOUS_VIDEO, falling back to AUTO, then OFF.
     * CONTINUOUS_PICTURE is far more responsive and locks sharp focus on subjects and human faces.
     */
    internal fun selectAfMode(characteristics: CameraCharacteristics): Int {
        val modes = characteristics.get(CameraCharacteristics.CONTROL_AF_AVAILABLE_MODES) ?: intArrayOf()
        return selectAfModeFromModes(modes)
    }

    /**
     * Selects optimal noise reduction mode: HIGH_QUALITY with FAST fallback.
     */
    internal fun resolveNoiseReductionMode(characteristics: CameraCharacteristics): Int? {
        val modes = characteristics.get(CameraCharacteristics.NOISE_REDUCTION_AVAILABLE_NOISE_REDUCTION_MODES) ?: intArrayOf()
        return resolveNoiseReductionModeFromModes(modes)
    }

    /**
     * Selects optimal edge enhancement mode: HIGH_QUALITY with FAST fallback.
     */
    internal fun resolveEdgeMode(characteristics: CameraCharacteristics): Int? {
        val modes = characteristics.get(CameraCharacteristics.EDGE_AVAILABLE_EDGE_MODES) ?: intArrayOf()
        return resolveEdgeModeFromModes(modes)
    }

    /**
     * Selects optimal hot pixel correction mode: HIGH_QUALITY with FAST fallback.
     */
    internal fun resolveHotPixelMode(characteristics: CameraCharacteristics): Int? {
        val modes = characteristics.get(CameraCharacteristics.HOT_PIXEL_AVAILABLE_HOT_PIXEL_MODES) ?: intArrayOf()
        return resolveHotPixelModeFromModes(modes)
    }

    /**
     * Selects hardware face detection mode: FULL if supported, falling back to SIMPLE.
     * Instructs camera ISP to automatically route autofocus and autoexposure to detected human faces.
     */
    internal fun resolveFaceDetectMode(characteristics: CameraCharacteristics): Int? {
        val modes = characteristics.get(CameraCharacteristics.STATISTICS_INFO_AVAILABLE_FACE_DETECT_MODES) ?: intArrayOf()
        return resolveFaceDetectModeFromModes(modes)
    }

    private fun createSession(
        camera: CameraDevice,
        targetSurface: Surface,
        targetSize: Size,
        resolved: ResolvedVideoConfig,
        onStarted: (ResolvedVideoConfig) -> Unit,
        onError: (String) -> Unit,
        previewSurface: Surface?,
        generation: Long,
        allowHighSpeed: Boolean = true,
    ) {
        try {
            if (generation != cameraGeneration) return
            // Kept so the session can be rebuilt around a viewfinder that appears
            // later, without reopening the camera.
            sessionRequest = SessionRequest(targetSurface, targetSize, resolved, onError)

            val characteristics = getCharacteristics(camera.id)
            val afMode = characteristics?.let { selectAfMode(it) } ?: CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE
            val antibandingMode = characteristics?.let { resolveAntibandingMode(it, configuredAntibanding) }
                ?: resolveAntibandingModeFallback(configuredAntibanding)
            val hasValidPreview = previewSurface != null && previewSurface.isValid

            // Query AE target FPS ranges. When the camera supports 60 FPS in its AE ranges
            // (e.g. [60, 60] or [30, 60]), use standard session configuration with
            // CONTROL_AE_TARGET_FPS_RANGE rather than constrained high speed.
            val streamMap = characteristics?.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            val aeRanges = characteristics?.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES) ?: emptyArray()
            val hasAe60 = aeRanges.any { it.upper >= 60 }
            val highSpeed = allowHighSpeed && !hasAe60 && resolved.fps >= 60 && streamMap?.highSpeedVideoSizes?.any { size ->
                size.width == targetSize.width && size.height == targetSize.height
            } == true

            // Constrained high-speed sessions strictly reject surfaces whose dimensions do
            // not match the high-speed video configuration (such as the UI TextureView).
            // Omit the viewfinder surface in high-speed mode so 60 FPS capture succeeds reliably.
            val attachPreview = hasValidPreview && !highSpeed

            val fpsRange = when {
                highSpeed -> selectHighSpeedRange(characteristics!!, targetSize)
                characteristics != null -> selectFpsRange(characteristics, resolved.fps)
                else -> Range(resolved.fps, resolved.fps)
            }

            val builder = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                addTarget(targetSurface)
                // The viewfinder is a second output of the same request. Adding it
                // to the repeating request rather than opening a second session is
                // what keeps preview and encode on the same frames: the user sees
                // exactly what the PC receives, with no extra camera open.
                if (attachPreview) previewSurface?.let { addTarget(it) }
                set(CaptureRequest.CONTROL_MODE, CaptureRequest.CONTROL_MODE_AUTO)
                if (manualFocusDistance != null) {
                    set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_OFF)
                    val maxFocus = characteristics?.get(CameraCharacteristics.LENS_INFO_MINIMUM_FOCUS_DISTANCE) ?: 10f
                    val clamped = manualFocusDistance!!.coerceIn(0f, maxFocus)
                    set(CaptureRequest.LENS_FOCUS_DISTANCE, clamped)
                } else {
                    set(CaptureRequest.CONTROL_AF_MODE, afMode)
                }
                set(
                    CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE,
                    // A floor below the target, not a lock at it.
                    //
                    // Pinning the range to `fps..fps` forbids the camera from
                    // lengthening its exposure, and a webcam is used indoors far
                    // more often than in daylight. When the room dims, or a moving
                    // subject needs a shorter exposure to stay sharp, the camera
                    // had no headroom: it either underexposed or held a frame back
                    // waiting for light, which reaches the viewer as a freeze in
                    // exactly the conditions the user reports (movement, a room
                    // that is not bright).
                    //
                    // Allowing the range to fall to half the target lets AE trade
                    // frame rate for exposure when it must, which is the same
                    // choice a phone camera makes for video. The encoder is told
                    // the target rate, so a dropped frame is a gap rather than a
                    // timing error.
                    fpsRange,
                )
                set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
                if (exposureCompensation != 0) {
                    val expRange = characteristics?.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE)
                    val clamped = if (expRange != null) exposureCompensation.coerceIn(expRange.lower, expRange.upper) else exposureCompensation
                    set(CaptureRequest.CONTROL_AE_EXPOSURE_COMPENSATION, clamped)
                }
                // Anti-banding at 50 Hz: the mains frequency here is 50 Hz, and
                // without this the camera's exposure beats against the lamps.
                // The visible symptom is a band rolling through the picture, and
                // the second-order one is AE hunting to compensate, which makes
                // every frame's brightness differ and reads as flicker. AUTO is
                // used when the device will decide for itself.
                set(
                    CaptureRequest.CONTROL_AE_ANTIBANDING_MODE,
                    antibandingMode,
                )
                if (whiteBalanceKelvin != null) {
                    set(CaptureRequest.CONTROL_AWB_MODE, CaptureRequest.CONTROL_AWB_MODE_OFF)
                    set(CaptureRequest.COLOR_CORRECTION_MODE, CaptureRequest.COLOR_CORRECTION_MODE_TRANSFORM_MATRIX)
                    set(CaptureRequest.COLOR_CORRECTION_TRANSFORM, identityColorTransform())
                    set(CaptureRequest.COLOR_CORRECTION_GAINS, kelvinToRggb(whiteBalanceKelvin!!))
                } else {
                    set(CaptureRequest.CONTROL_AWB_MODE, CaptureRequest.CONTROL_AWB_MODE_AUTO)
                }
                set(CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE, CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_OFF)
                if (torchEnabled) {
                    set(CaptureRequest.FLASH_MODE, CaptureRequest.FLASH_MODE_TORCH)
                }
                // Minimize video noise: high quality noise reduction, edge enhancement, and hot pixel correction
                if (characteristics != null) {
                    resolveNoiseReductionMode(characteristics)?.let { mode ->
                        set(CaptureRequest.NOISE_REDUCTION_MODE, mode)
                    }
                    resolveEdgeMode(characteristics)?.let { mode ->
                        set(CaptureRequest.EDGE_MODE, mode)
                    }
                    resolveHotPixelMode(characteristics)?.let { mode ->
                        set(CaptureRequest.HOT_PIXEL_MODE, mode)
                    }
                    // Hardware face detection: route ISP AF/AE to user's face in front of camera
                    resolveFaceDetectMode(characteristics)?.let { mode ->
                        set(CaptureRequest.STATISTICS_FACE_DETECT_MODE, mode)
                    }
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && characteristics != null) {
                    val zoomRange = characteristics.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)
                    if (zoomRange != null) {
                        val clamped = zoom.coerceIn(zoomRange.lower, zoomRange.upper)
                        set(CaptureRequest.CONTROL_ZOOM_RATIO, clamped)
                    } else if (zoom > 1f) {
                        computeCropRect(characteristics, zoom)?.let { crop ->
                            set(CaptureRequest.SCALER_CROP_REGION, crop)
                        }
                    }
                } else if (zoom > 1f && characteristics != null) {
                    computeCropRect(characteristics, zoom)?.let { crop ->
                        set(CaptureRequest.SCALER_CROP_REGION, crop)
                    }
                }
            }
            requestBuilder = builder

            val surfaces = buildList {
                add(targetSurface)
                if (attachPreview) previewSurface?.let { add(it) }
            }
            val outputs = surfaces.map { OutputConfiguration(it) }
            // The executor must hold the handler this session was built on, not
            // read the field at call time.
            //
            // `stop()` clears `handler` and quits the thread, and Camera2 still
            // delivers the session's `onClosed` afterwards. Reading the field
            // there meant `handler?.post(...)` became a no-op, so the callback
            // was silently dropped and Camera2 logged "sending message to a
            // Handler on a dead thread" on every teardown. Capturing the handler
            // keeps the callbacks addressed to the looper that owns them; once
            // that looper has quit the post is rejected inside the message
            // queue rather than dereferencing a field another thread just nulled.
            val sessionHandler = handler
            val callbackExecutor = java.util.concurrent.Executor { command ->
                if (sessionHandler == null) {
                    // No looper to run on: run inline rather than losing the
                    // callback. Camera2 requires every callback to be delivered.
                    command.run()
                } else {
                    sessionHandler.post(command)
                }
            }

            val stateCallback = object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(configured: CameraCaptureSession) {
                    if (generation != cameraGeneration) {
                        configured.close()
                        return
                    }
                    session = configured
                    try {
                        if (highSpeed) {
                            // A high speed session only accepts the burst API, and
                            // the burst must be built by the session itself.
                            val speed = configured as CameraConstrainedHighSpeedCaptureSession
                            speed.setRepeatingBurst(
                                speed.createHighSpeedRequestList(builder.build()),
                                null,
                                handler,
                            )
                        } else {
                            configured.setRepeatingRequest(builder.build(), null, handler)
                        }
                        Log.i(
                            TAG,
                            "streaming ${targetSize.width}x${targetSize.height}@${resolved.fps} " +
                                "(${resolved.bitrate}bps, fpsRange=$fpsRange, highSpeed=$highSpeed, preview=$attachPreview) " +
                                "camera=${camera.id}",
                        )
                        onStarted(resolved)
                    } catch (t: Throwable) {
                        Log.e(TAG, "setRepeatingRequest failed", t)
                        onError("The camera started but the video stream didn't. Try stopping and starting again.")
                    }
                }

                override fun onConfigureFailed(configured: CameraCaptureSession) {
                    if (attachPreview) {
                        Log.w(TAG, "createCaptureSession failed with preview surface; retrying with encoder target only")
                        createSession(
                            camera = camera,
                            targetSurface = targetSurface,
                            targetSize = targetSize,
                            resolved = resolved,
                            onStarted = onStarted,
                            onError = onError,
                            previewSurface = null,
                            generation = generation,
                            allowHighSpeed = allowHighSpeed,
                        )
                        return
                    }
                    if (highSpeed) {
                        Log.w(TAG, "constrained high-speed session failed for ${targetSize.width}x${targetSize.height}; falling back to regular session")
                        val fallbackFps = resolved.fps.coerceAtMost(30)
                        createSession(
                            camera = camera,
                            targetSurface = targetSurface,
                            targetSize = targetSize,
                            resolved = resolved.copy(fps = fallbackFps, substituted = true),
                            onStarted = onStarted,
                            onError = onError,
                            previewSurface = previewSurface,
                            generation = generation,
                            allowHighSpeed = false,
                        )
                        return
                    }
                    if (generation != cameraGeneration) return
                    onError(
                        "This phone can't stream ${targetSize.width}x${targetSize.height}. " +
                            "Try a lower resolution.",
                    )
                }
            }

            if (highSpeed) {
                try {
                    @Suppress("DEPRECATION")
                    camera.createConstrainedHighSpeedCaptureSession(surfaces, stateCallback, handler)
                } catch (hsErr: Throwable) {
                    Log.w(TAG, "createConstrainedHighSpeedCaptureSession threw ${hsErr.javaClass.simpleName}; falling back to regular session", hsErr)
                    val fallbackFps = resolved.fps.coerceAtMost(30)
                    createSession(
                        camera = camera,
                        targetSurface = targetSurface,
                        targetSize = targetSize,
                        resolved = resolved.copy(fps = fallbackFps, substituted = true),
                        onStarted = onStarted,
                        onError = onError,
                        previewSurface = previewSurface,
                        generation = generation,
                        allowHighSpeed = false,
                    )
                }
            } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                val config = SessionConfiguration(
                    SessionConfiguration.SESSION_REGULAR,
                    outputs,
                    callbackExecutor,
                    stateCallback,
                )
                camera.createCaptureSession(config)
            } else {
                @Suppress("DEPRECATION")
                camera.createCaptureSession(surfaces, stateCallback, handler)
            }
        } catch (t: Throwable) {
            Log.e(TAG, "createSession failed", t)
            if (generation != cameraGeneration) return
            onError("Couldn't configure the camera for streaming.")
        }
    }

    /** Torch is a capability-gated control (PRD §22): a phone without a flash ignores it. */
    fun setTorch(enabled: Boolean) {
        Log.i(TAG, "torch request enabled=$enabled at uptimeMs=${android.os.SystemClock.elapsedRealtime()}")
        torchEnabled = enabled
        val builder = requestBuilder ?: return
        runCatching {
            builder.set(CaptureRequest.FLASH_MODE, if (enabled) CaptureRequest.FLASH_MODE_TORCH else CaptureRequest.FLASH_MODE_OFF)
            applyRequest(builder)
        }.onFailure { Log.d(TAG, "torch not supported: ${it.javaClass.simpleName}") }
    }

    private fun computeCropRect(characteristics: CameraCharacteristics, zoomFactor: Float): Rect? {
        val maxZoom = characteristics.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM) ?: 1f
        if (maxZoom <= 1.0f) return null
        val clamped = zoomFactor.coerceIn(1f, maxZoom)
        val sensorRect = characteristics.get(CameraCharacteristics.SENSOR_INFO_ACTIVE_ARRAY_SIZE) ?: return null
        if (clamped <= 1.001f) {
            zoom = 1f
            return sensorRect
        }
        zoom = clamped
        // Bayer CFA (Color Filter Array) and hardware scalers require coordinates aligned to 2 (even numbers)
        val rawWidth = (sensorRect.width() / clamped).toInt().coerceAtLeast(2)
        val rawHeight = (sensorRect.height() / clamped).toInt().coerceAtLeast(2)
        val cropWidth = (rawWidth / 2) * 2
        val cropHeight = (rawHeight / 2) * 2
        val rawLeft = sensorRect.left + (sensorRect.width() - cropWidth) / 2
        val rawTop = sensorRect.top + (sensorRect.height() - cropHeight) / 2
        val left = (rawLeft / 2) * 2
        val top = (rawTop / 2) * 2
        val right = (left + cropWidth).coerceAtMost(sensorRect.right)
        val bottom = (top + cropHeight).coerceAtMost(sensorRect.bottom)
        return Rect(left, top, right, bottom)
    }

    /**
     * Digital zoom via a crop region or Android 11+ zoom ratio.
     *
     * Clamped to the sensor's reported maximum so a request beyond the hardware
     * cannot silently produce an invalid frame or HAL crash.
     */
    fun setZoom(cameraId: String, zoomFactor: Float) {
        val characteristics = getCharacteristics(cameraId) ?: return
        val builder = requestBuilder

        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                val zoomRange = characteristics.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)
                if (zoomRange != null) {
                    val clamped = zoomFactor.coerceIn(zoomRange.lower, zoomRange.upper)
                    zoom = clamped
                    if (builder != null) {
                        builder.set(CaptureRequest.CONTROL_ZOOM_RATIO, clamped)
                        applyRequest(builder)
                    }
                    return
                }
            }

            val maxZoom = characteristics.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM) ?: 1f
            if (maxZoom <= 1.0f) {
                zoom = 1f
                return
            }

            val crop = computeCropRect(characteristics, zoomFactor) ?: return
            if (builder != null) {
                builder.set(CaptureRequest.SCALER_CROP_REGION, crop)
                applyRequest(builder)
            }
        }.onFailure { Log.d(TAG, "zoom not supported: ${it.javaClass.simpleName}") }
    }

    /**
     * Sets exposure compensation in EV index steps.
     * Clamped to [CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE].
     */
    fun setExposure(compensation: Int) {
        exposureCompensation = compensation
        val builder = requestBuilder ?: return
        val characteristics = device?.id?.let { getCharacteristics(it) }
        val range = characteristics?.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE)
        val clamped = if (range != null) compensation.coerceIn(range.lower, range.upper) else compensation
        runCatching {
            builder.set(CaptureRequest.CONTROL_AE_EXPOSURE_COMPENSATION, clamped)
            applyRequest(builder)
            Log.i(TAG, "Applied exposure compensation: $clamped (requested $compensation)")
        }.onFailure { Log.d(TAG, "setExposure failed: ${it.javaClass.simpleName}") }
    }

    /**
     * Sets manual or auto white balance.
     * When [kelvin] is provided (e.g. 2000K..10000K), sets CONTROL_AWB_MODE to OFF
     * and applies custom color gains via COLOR_CORRECTION_GAINS.
     * When [kelvin] is null, returns CONTROL_AWB_MODE to AUTO.
     */
    fun setWhiteBalance(kelvin: Int?) {
        whiteBalanceKelvin = kelvin
        val builder = requestBuilder ?: return
        runCatching {
            if (kelvin == null) {
                builder.set(CaptureRequest.CONTROL_AWB_MODE, CaptureRequest.CONTROL_AWB_MODE_AUTO)
                builder.set(CaptureRequest.COLOR_CORRECTION_MODE, CaptureRequest.COLOR_CORRECTION_MODE_FAST)
            } else {
                builder.set(CaptureRequest.CONTROL_AWB_MODE, CaptureRequest.CONTROL_AWB_MODE_OFF)
                builder.set(CaptureRequest.COLOR_CORRECTION_MODE, CaptureRequest.COLOR_CORRECTION_MODE_TRANSFORM_MATRIX)
                builder.set(CaptureRequest.COLOR_CORRECTION_TRANSFORM, identityColorTransform())
                builder.set(CaptureRequest.COLOR_CORRECTION_GAINS, kelvinToRggb(kelvin))
            }
            applyRequest(builder)
            Log.i(TAG, "Applied white balance: ${kelvin?.let { "${it}K" } ?: "AUTO"}")
        }.onFailure { Log.d(TAG, "setWhiteBalance failed: ${it.javaClass.simpleName}") }
    }

    /**
     * Sets manual focus distance in diopters (1 / distance in meters; 0.0f = infinity).
     * When [diopters] is provided, sets CONTROL_AF_MODE to OFF and applies LENS_FOCUS_DISTANCE.
     * When [diopters] is null, returns CONTROL_AF_MODE to CONTINUOUS_PICTURE or best supported AF mode.
     */
    fun setFocusDistance(diopters: Float?) {
        manualFocusDistance = diopters
        val builder = requestBuilder ?: return
        val characteristics = device?.id?.let { getCharacteristics(it) }
        val defaultAfMode = characteristics?.let { selectAfMode(it) } ?: CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE
        runCatching {
            if (diopters == null) {
                builder.set(CaptureRequest.CONTROL_AF_MODE, defaultAfMode)
            } else {
                builder.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_OFF)
                val maxFocus = characteristics?.get(CameraCharacteristics.LENS_INFO_MINIMUM_FOCUS_DISTANCE) ?: 10f
                val clamped = diopters.coerceIn(0f, maxFocus)
                builder.set(CaptureRequest.LENS_FOCUS_DISTANCE, clamped)
            }
            applyRequest(builder)
            Log.i(TAG, "Applied focus distance: ${diopters?.let { "${it} diopters" } ?: "AUTO"}")
        }.onFailure { Log.d(TAG, "setFocusDistance failed: ${it.javaClass.simpleName}") }
    }

    /** Re-applies the requested frame rate without tearing down the session. */
    fun setFrameRate(fps: Int) {
        val builder = requestBuilder ?: return
        val characteristics = device?.id?.let { getCharacteristics(it) }
        val activeSession = session
        val range = if (activeSession is CameraConstrainedHighSpeedCaptureSession && characteristics != null) {
            val size = sessionRequest?.targetSize
            if (size != null) selectHighSpeedRange(characteristics, size) else Range(fps, fps)
        } else {
            characteristics?.let { selectFpsRange(it, fps) } ?: Range(fps, fps)
        }
        runCatching {
            builder.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, range)
            applyRequest(builder)
        }.onFailure { Log.d(TAG, "fps change rejected: ${it.javaClass.simpleName}") }
    }

    /**
     * Attaches the on-screen viewfinder to the live capture session.
     *
     * Safe to call before a stream starts (the surface is used when the session
     * is built) and while one is running (the session is rebuilt around the extra
     * output). Rebuilding is cheap and keeps the camera open, which matters
     * because closing and reopening it would show a black gap on the phone.
     */
    fun setPreviewSurface(surface: Surface?) {
        if (previewSurface === surface) return
        previewSurface = surface

        val cameraHandler = handler
        val rebuild = Runnable {
            if (this@CameraController.previewSurface !== surface) return@Runnable
            // Constrained high-speed sessions omit the preview surface so Camera2
            // never rejects mismatched viewfinder sizes; avoid disrupting the live 60 FPS stream.
            if (session is CameraConstrainedHighSpeedCaptureSession) return@Runnable
            val request = sessionRequest ?: return@Runnable
            val camera = device ?: return@Runnable
            // Camera2 does not allow overlapping capture sessions on one device.
            // Close the old output set before creating the replacement; otherwise a
            // TextureView attach/detach can make the HAL report ERROR_CAMERA_DEVICE.
            cameraGeneration++
            runCatching { session?.close() }
            session = null
            // Rebuild around the new output set. `onStarted` is not re-invoked: the
            // stream configuration did not change, only where frames are mirrored.
            createSession(
                camera = camera,
                targetSurface = request.targetSurface,
                targetSize = request.targetSize,
                resolved = request.resolved,
                onStarted = {},
                onError = request.onError,
                previewSurface = surface,
                generation = cameraGeneration,
            )
        }
        if (cameraHandler != null) {
            cameraHandler.post(rebuild)
        } else {
            rebuild.run()
        }
    }

    fun stop() {
        closeDevice()
    }

    fun release() {
        closeDevice()
        stopThread()
    }

    /**
     * Closes the open camera and its capture session, leaving the handler thread
     * running so the next camera can be opened on it.
     *
     * Manual pro controls (EV, WB, Focus) are preserved across camera switches
     * and stream restarts unless explicitly reset.
     */
    private fun closeDevice() {
        cameraGeneration++
        runCatching { session?.close() }
        runCatching { device?.close() }
        session = null
        device = null
        requestBuilder = null
        activeConfig = null
        sessionRequest = null
    }

    val torchState: Boolean
        get() = torchEnabled

    val zoomFactor: Float
        get() = zoom

    private fun startThread() {
        // A switch keeps the thread it already has; see `start(keepThread = true)`.
        if (thread?.isAlive == true && handler != null) return
        stopThread()
        val t = HandlerThread("phonecam-camera").apply { start() }
        thread = t
        handler = Handler(t.looper)
    }

    private fun stopThread() {
        val oldThread = thread
        if (oldThread == null) {
            handler = null
            return
        }
        runCatching {
            oldThread.quitSafely()
            oldThread.join(1000L)
        }.onFailure { Log.w(TAG, "camera thread shutdown did not complete cleanly", it) }
        thread = null
        handler = null
    }

    /**
     * Inspects StreamConfigurationMap and AE ranges for the max FPS achievable
     * at [width]x[height] for [cameraId] (e.g. 60 or 30).
     */
    fun maxFpsForSize(cameraId: String, width: Int, height: Int): Int {
        val characteristics = getCharacteristics(cameraId) ?: return 30
        val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return 30
        val size = Size(width, height)
        val durationNs = map.getOutputMinFrameDuration(SurfaceTexture::class.java, size)
            .takeIf { it > 0 }
            ?: map.getOutputMinFrameDuration(android.media.MediaRecorder::class.java, size)
        val fpsFromDuration = if (durationNs > 0) (1_000_000_000L / durationNs).toInt() else 30

        val aeRanges = characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
        val aeMax = aeRanges?.maxOfOrNull { it.upper } ?: 30

        val hsRanges = runCatching { map.getHighSpeedVideoFpsRangesFor(size) }.getOrNull()
        val hsMax = hsRanges?.maxOfOrNull { it.upper } ?: 0

        return maxOf(minOf(fpsFromDuration, aeMax), hsMax, if (aeMax >= 60) 60 else 30)
    }

    companion object {
        const val MAX_VIDEO_WIDTH = 3840
        const val MAX_VIDEO_HEIGHT = 2160
        const val TAG = "CameraController"

        private fun identityColorTransform(): ColorSpaceTransform =
            ColorSpaceTransform(intArrayOf(1, 1, 0, 1, 0, 1, 0, 1, 1, 1, 0, 1, 0, 1, 0, 1, 1, 1))

        fun kelvinToColorGains(kelvin: Int): ColorGains {
            val k = kelvin.coerceIn(2000, 10000).toFloat()
            val rGain: Float
            val bGain: Float
            if (k < 5000f) {
                val t = (k - 2000f) / 3000f
                rGain = 1.0f + 0.3f * t
                bGain = 2.8f - 1.2f * t
            } else {
                val t = (k - 5000f) / 5000f
                rGain = 1.3f + 1.2f * t
                bGain = 1.6f - 0.6f * t
            }
            return ColorGains(rGain.coerceIn(0.5f, 4.0f), 1.0f, 1.0f, bGain.coerceIn(0.5f, 4.0f))
        }

        fun kelvinToRggb(kelvin: Int): RggbChannelVector = kelvinToColorGains(kelvin).toRggbVector()

        fun selectHighSpeedRangeFromPairs(ranges: List<Pair<Int, Int>>): Pair<Int, Int> {
            if (ranges.isEmpty()) return 30 to 120

            // 1. Exact fixed [60, 60]
            ranges.firstOrNull { it.first == 60 && it.second == 60 }?.let { return it }

            // 2. Variable [30, 60] (or any upper == 60, preferring highest lower <= 60)
            val upper60 = ranges.filter { it.second == 60 }
            if (upper60.isNotEmpty()) {
                return upper60.firstOrNull { it.first == 30 }
                    ?: upper60.maxByOrNull { it.first }!!
            }

            // 3. Any range supporting >= 60: pick lowest upper >= 60 (e.g. 120 over 240),
            // preferring variable lower == 30 (e.g. [30, 120] over [120, 120]) for indoor AE headroom.
            val atLeast60 = ranges.filter { it.second >= 60 }
            if (atLeast60.isNotEmpty()) {
                val minUpper = atLeast60.minOf { it.second }
                val candidates = atLeast60.filter { it.second == minUpper }
                return candidates.firstOrNull { it.first == 30 }
                    ?: candidates.minByOrNull { it.first }!!
            }

            return ranges.maxByOrNull { it.second } ?: (30 to 120)
        }

        fun selectFpsRangeFromPairs(ranges: List<Pair<Int, Int>>, targetFps: Int): Pair<Int, Int> {
            if (targetFps >= 60) {
                val matching60 = ranges.filter { it.second == 60 }
                if (matching60.isNotEmpty()) {
                    val preferred = matching60.firstOrNull { it.first == 60 }
                        ?: matching60.firstOrNull { it.first == 30 }
                        ?: matching60.first()
                    return preferred
                }
            }

            val matchingUpper = ranges.filter { it.second == targetFps }
            if (matchingUpper.isNotEmpty()) {
                // Prioritize fixed range [targetFps, targetFps] (e.g. [30, 30])
                val exact = matchingUpper.firstOrNull { it.first == targetFps }
                if (exact != null) {
                    return exact
                }
                // Fallback to closest variable range (highest lower bound)
                return matchingUpper.maxByOrNull { it.first } ?: matchingUpper.first()
            }

            val containing = ranges.filter { targetFps in it.first..it.second }
            if (containing.isNotEmpty()) {
                return containing.minByOrNull { it.second - it.first }!!
            }

            val highest = ranges.maxByOrNull { it.second }
            if (highest != null && highest.second < targetFps) {
                return highest
            }

            return targetFps to targetFps
        }

        fun resolveAntibandingModeFallback(requested: String): Int = when (requested.lowercase()) {
            "50hz", "50" -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_50HZ
            "60hz", "60" -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_60HZ
            "off" -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_OFF
            else -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_AUTO
        }

        fun selectAfModeFromModes(modes: IntArray): Int = when {
            modes.contains(CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE) -> CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE
            modes.contains(CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO) -> CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO
            modes.contains(CaptureRequest.CONTROL_AF_MODE_AUTO) -> CaptureRequest.CONTROL_AF_MODE_AUTO
            else -> CaptureRequest.CONTROL_AF_MODE_OFF
        }

        fun resolveNoiseReductionModeFromModes(modes: IntArray): Int? = when {
            modes.contains(CaptureRequest.NOISE_REDUCTION_MODE_HIGH_QUALITY) -> CaptureRequest.NOISE_REDUCTION_MODE_HIGH_QUALITY
            modes.contains(CaptureRequest.NOISE_REDUCTION_MODE_FAST) -> CaptureRequest.NOISE_REDUCTION_MODE_FAST
            else -> null
        }

        fun resolveEdgeModeFromModes(modes: IntArray): Int? = when {
            modes.contains(CaptureRequest.EDGE_MODE_HIGH_QUALITY) -> CaptureRequest.EDGE_MODE_HIGH_QUALITY
            modes.contains(CaptureRequest.EDGE_MODE_FAST) -> CaptureRequest.EDGE_MODE_FAST
            else -> null
        }

        fun resolveHotPixelModeFromModes(modes: IntArray): Int? = when {
            modes.contains(CaptureRequest.HOT_PIXEL_MODE_HIGH_QUALITY) -> CaptureRequest.HOT_PIXEL_MODE_HIGH_QUALITY
            modes.contains(CaptureRequest.HOT_PIXEL_MODE_FAST) -> CaptureRequest.HOT_PIXEL_MODE_FAST
            else -> null
        }

        fun resolveFaceDetectModeFromModes(modes: IntArray): Int? = when {
            modes.contains(CaptureRequest.STATISTICS_FACE_DETECT_MODE_FULL) -> CaptureRequest.STATISTICS_FACE_DETECT_MODE_FULL
            modes.contains(CaptureRequest.STATISTICS_FACE_DETECT_MODE_SIMPLE) -> CaptureRequest.STATISTICS_FACE_DETECT_MODE_SIMPLE
            else -> null
        }
    }
}
