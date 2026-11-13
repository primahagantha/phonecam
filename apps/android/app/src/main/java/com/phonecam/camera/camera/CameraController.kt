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
import android.hardware.camera2.params.OutputConfiguration
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
    private var activeConfig: ResolvedVideoConfig? = null

    /** The size the camera is really producing, which may differ from the request. */
    val resolvedConfig: ResolvedVideoConfig?
        get() = activeConfig

    /** Sizes this camera can capture in a high speed session, which is where 60 fps lives. */
    fun highSpeedSizes(cameraId: String): List<Pair<Int, Int>> {
        val map = getCharacteristics(cameraId)
            ?.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return emptyList()
        return map.highSpeedVideoSizes?.map { it.width to it.height } ?: emptyList()
    }

    /**
     * Frame rates this camera will accept.
     *
     * 24 to 30 come from the AE ranges. 60 does not, and never will: on devices
     * that have it, 60 is a high speed configuration outside the AE range list
     * (this one reports `[13,30] [30,30]` and `1280x720 30-120`). Offering it
     * from AE alone is why the 60 option never appeared.
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
        return (normal + if (supportsHighSpeedFps(characteristics, 60)) listOf(60) else emptyList())
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
    /** Output sizes for encoder surfaces, with a metadata fallback for devices that
     * omit the generic Surface class but advertise the same outputs as a texture. */
    private fun outputSizes(map: android.hardware.camera2.params.StreamConfigurationMap): Array<Size> {
        val surfaceSizes = map.getOutputSizes(Surface::class.java)
        if (!surfaceSizes.isNullOrEmpty()) return surfaceSizes
        return map.getOutputSizes(SurfaceTexture::class.java) ?: emptyArray()
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
         * Keep the camera handler thread alive across the teardown.
         *
         * A camera switch closes one device and opens another on the same
         * thread. Quitting that thread and starting a replacement costs a
         * `quitSafely` drain plus a join — a quarter of a second on this phone,
         * which the viewer sees as the freeze at the start of every switch.
         */
        keepThread: Boolean = false,
    ) {
        if (keepThread && thread != null) {
            closeDevice()
        } else {
            stop()
        }
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
        )
        activeConfig = resolved

        val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
        val exact = map?.let { outputSizes(it) }?.firstOrNull { size ->
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
     * The upper bound is the target, so good light gives exactly the rate asked
     * for. The lower bound is chosen as wide as the device allows down to half
     * the target: pinning the range to `fps..fps` forbids the camera from
     * lengthening its exposure, and a webcam is used indoors far more often than
     * in daylight. When the room dims, a locked range leaves AE no headroom, so
     * it hunts — which reaches the viewer as flicker — or holds frames back,
     * which reaches them as a stutter. A floor at half the target lets AE trade
     * rate for exposure only when it must, the same choice a phone camera makes
     * for video.
     */
    private fun selectFpsRange(characteristics: CameraCharacteristics, targetFps: Int): Range<Int> {
        val ranges = characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES) ?: emptyArray()

        val matchingUpper = ranges.filter { it.upper == targetFps }
        if (matchingUpper.isNotEmpty()) {
            val withHeadroom = matchingUpper.filter { it.lower >= targetFps / 2 }
            return withHeadroom.minByOrNull { it.lower } ?: matchingUpper.minByOrNull { it.lower }!!
        }

        val containing = ranges.filter { it.contains(targetFps) }
        if (containing.isNotEmpty()) {
            return containing.minByOrNull { it.upper - it.lower }!!
        }

        return Range(targetFps, targetFps)
    }

    /**
     * Frame rate range for a high speed session.
     *
     * Unlike a normal session, Camera2 only accepts a range the high speed
     * configuration itself advertises. The narrowest one that still reaches 60 is
     * chosen, so a device that also offers `[60,60]` is not run at its 120 fps
     * maximum for a 60 fps request.
     */
    private fun selectHighSpeedRange(characteristics: CameraCharacteristics, size: Size): Range<Int> {
        val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
        val ranges = map?.getHighSpeedVideoFpsRangesFor(size)
        return ranges?.filter { it.upper >= 60 }?.minByOrNull { it.upper }
            ?: ranges?.maxByOrNull { it.upper }
            ?: Range(30, 120)
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

    /**
     * Selects antibanding mode. Prefers AUTO when supported so the camera HAL
     * dynamically detects 50 Hz vs 60 Hz flicker (eliminating lighting and monitor
     * beat flicker), falling back to 50 Hz then 60 Hz.
     */
    private fun selectAntibandingMode(characteristics: CameraCharacteristics): Int {
        val modes = characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_ANTIBANDING_MODES) ?: intArrayOf()
        return when {
            modes.contains(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_AUTO) -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_AUTO
            modes.contains(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_50HZ) -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_50HZ
            modes.contains(CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_60HZ) -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_60HZ
            else -> CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_OFF
        }
    }

    /**
     * Safely selects autofocus mode based on camera capabilities.
     * Front cameras are typically fixed-focus and don't support CONTINUOUS_VIDEO or AUTO.
     */
    private fun selectAfMode(characteristics: CameraCharacteristics): Int {
        val modes = characteristics.get(CameraCharacteristics.CONTROL_AF_AVAILABLE_MODES) ?: intArrayOf()
        return when {
            modes.contains(CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO) -> CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO
            modes.contains(CaptureRequest.CONTROL_AF_MODE_AUTO) -> CaptureRequest.CONTROL_AF_MODE_AUTO
            else -> CaptureRequest.CONTROL_AF_MODE_OFF
        }
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
    ) {
        try {
            if (generation != cameraGeneration) return
            // Kept so the session can be rebuilt around a viewfinder that appears
            // later, without reopening the camera.
            sessionRequest = SessionRequest(targetSurface, targetSize, resolved, onError)

            val characteristics = getCharacteristics(camera.id)
            val afMode = characteristics?.let { selectAfMode(it) } ?: CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO
            val antibandingMode = characteristics?.let { selectAntibandingMode(it) } ?: CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_50HZ
            val hasValidPreview = previewSurface != null && previewSurface.isValid

            // 60 fps and above is a *constrained high speed* session on Camera2. It
            // has its own creation call, its own repeating-burst API, and its own
            // frame rate ranges; the AE ranges used by a normal session are
            // rejected outright. The encoder surface is already sized to the
            // resolved config, which the resolver snapped to a high speed size.
            val streamMap = characteristics?.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            val highSpeed = resolved.fps >= 60 && streamMap?.highSpeedVideoSizes?.any { size ->
                size.width == targetSize.width && size.height == targetSize.height
            } == true
            val fpsRange = when {
                highSpeed && characteristics != null ->
                    selectHighSpeedRange(characteristics, targetSize)
                characteristics != null -> selectFpsRange(characteristics, resolved.fps)
                else -> Range(resolved.fps, resolved.fps)
            }

            val builder = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                addTarget(targetSurface)
                // The viewfinder is a second output of the same request. Adding it
                // to the repeating request rather than opening a second session is
                // what keeps preview and encode on the same frames: the user sees
                // exactly what the PC receives, with no extra camera open.
                if (hasValidPreview) previewSurface?.let { addTarget(it) }
                set(CaptureRequest.CONTROL_MODE, CaptureRequest.CONTROL_MODE_AUTO)
                set(
                    CaptureRequest.CONTROL_AF_MODE,
                    afMode,
                )
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
                set(CaptureRequest.CONTROL_AWB_MODE, CaptureRequest.CONTROL_AWB_MODE_AUTO)
                set(CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE, CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_OFF)
                if (torchEnabled) {
                    set(CaptureRequest.FLASH_MODE, CaptureRequest.FLASH_MODE_TORCH)
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
                if (hasValidPreview) previewSurface?.let { add(it) }
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
                                "(${resolved.bitrate}bps) camera=${camera.id}",
                        )
                        onStarted(resolved)
                    } catch (t: Throwable) {
                        Log.e(TAG, "setRepeatingRequest failed", t)
                        onError("The camera started but the video stream didn't. Try stopping and starting again.")
                    }
                }

                override fun onConfigureFailed(configured: CameraCaptureSession) {
                    if (hasValidPreview) {
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
                @Suppress("DEPRECATION")
                camera.createConstrainedHighSpeedCaptureSession(surfaces, stateCallback, handler)
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
        val builder = requestBuilder ?: return
        val characteristics = getCharacteristics(cameraId) ?: return
        val maxZoom = characteristics.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM) ?: 1f
        if (maxZoom <= 1.0f) {
            zoom = 1f
            return
        }

        runCatching {
            val sensorRect = characteristics.get(CameraCharacteristics.SENSOR_INFO_ACTIVE_ARRAY_SIZE)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                val zoomRange = characteristics.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)
                if (zoomRange != null) {
                    val clamped = zoomFactor.coerceIn(zoomRange.lower, zoomRange.upper)
                    zoom = clamped
                    builder.set(CaptureRequest.CONTROL_ZOOM_RATIO, clamped)
                    applyRequest(builder)
                    return
                }
            }

            val crop = computeCropRect(characteristics, zoomFactor) ?: return
            builder.set(CaptureRequest.SCALER_CROP_REGION, crop)
            applyRequest(builder)
        }.onFailure { Log.d(TAG, "zoom not supported: ${it.javaClass.simpleName}") }
    }

    /** Re-applies the requested frame rate without tearing down the session. */
    fun setFrameRate(fps: Int) {
        val builder = requestBuilder ?: return
        runCatching {
            builder.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(fps, fps))
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
        stopThread()
    }

    /**
     * Closes the open camera and its capture session, leaving the handler thread
     * running so the next camera can be opened on it.
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
        torchEnabled = false
        zoom = 1f
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

    private companion object {
        const val MAX_VIDEO_WIDTH = 1920
        const val MAX_VIDEO_HEIGHT = 1080
        const val TAG = "CameraController"
    }
}
