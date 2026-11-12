package com.phonecam.camera.camera

import android.content.Context
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.hardware.camera2.params.StreamConfigurationMap
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.util.Log
import android.util.Size
import com.phonecam.camera.protocol.CameraInfo

/**
 * Runtime camera capability detection (PRD §44, Rule 5).
 *
 * Everything the UI is allowed to offer comes from here. Nothing is assumed
 * about which resolutions, frame rates, or controls a device has; the reported
 * list is the intersection of what the camera sensor advertises and what the
 * hardware H.264 encoder can accept, because a size the encoder rejects is not
 * usable no matter what the sensor says.
 */
class CameraCapabilityProbe(private val context: Context) {

    private val cameraManager =
        context.getSystemService(Context.CAMERA_SERVICE) as CameraManager

    /**
     * Enumerates cameras, sorted so the rear camera is first (the primary
     * use case) and the front camera second.
     */
    fun probe(): List<CameraInfo> {
        val encoder = EncoderCapability.query()
        val result = mutableListOf<CameraInfo>()

        for (id in cameraManager.cameraIdList) {
            val info = runCatching { describeCamera(id, encoder) }.getOrNull()
            if (info != null) result += info
        }

        return result.sortedWith(
            compareBy(
                { if (it.facing == "back") 0 else if (it.facing == "front") 1 else 2 },
                { it.id },
            ),
        )
    }

    private fun describeCamera(id: String, encoder: EncoderCapability): CameraInfo? {
        val characteristics = cameraManager.getCameraCharacteristics(id)
        val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            ?: return null

        val sizes = captureSizes(map)
        val encoderSizes = if (encoder.supportedSizes.isEmpty()) sizes else {
            sizes.filter { size ->
                encoderSizesContains(encoder.supportedSizes, size)
            }.ifEmpty { sizes }
        }

        // Only sizes that are a sane aspect ratio and at least 480p are offered.
        val offered = encoderSizes
            .filter { (w, h) -> w >= 320 && h >= 240 }
            .map { (w, h) -> if (w >= h) w to h else h to w }
            .distinct()
            .sortedByDescending { it.first.toLong() * it.second }

        val fps = supportedFrameRates(map, characteristics)
        val facing = when (characteristics.get(CameraCharacteristics.LENS_FACING)) {
            CameraCharacteristics.LENS_FACING_BACK -> "back"
            CameraCharacteristics.LENS_FACING_FRONT -> "front"
            CameraCharacteristics.LENS_FACING_EXTERNAL -> "external"
            else -> "unknown"
        }

        val maxZoom = characteristics.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM)
            ?.toFloat() ?: 1f
        val afModes = characteristics.get(CameraCharacteristics.CONTROL_AF_AVAILABLE_MODES)
        val focus = afModes?.any {
            it == CameraCharacteristics.CONTROL_AF_MODE_CONTINUOUS_VIDEO ||
                it == CameraCharacteristics.CONTROL_AF_MODE_AUTO
        } == true
        val awbModes = characteristics.get(CameraCharacteristics.CONTROL_AWB_AVAILABLE_MODES)
        val whiteBalance = awbModes?.any {
            it == CameraCharacteristics.CONTROL_AWB_MODE_AUTO
        } == true
        val exposure = characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_MODES)?.isNotEmpty() == true
        val torch = characteristics.get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true
        val sensorOrientation = characteristics.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 0

        return CameraInfo(
            id = id,
            facing = facing,
            resolutions = offered.map { (w, h) -> "${w}x$h" },
            fps = fps,
            torch = torch,
            focus = focus,
            zoom = maxZoom > 1f,
            exposure = exposure,
            whiteBalance = whiteBalance,
            minZoom = 1f,
            maxZoom = maxZoom.coerceAtLeast(1f),
            orientationDegrees = sensorOrientation,
        )
    }

    private fun captureSizes(map: StreamConfigurationMap): List<Pair<Int, Int>> {
        val sizes: Array<Size> = map.getOutputSizes(android.graphics.SurfaceTexture::class.java)
            ?: map.getOutputSizes(MediaCodecInfo.CodecCapabilities::class.java)
            ?: emptyArray()
        return sizes.map { it.width to it.height }
    }

    /**
     * Frame rates are derived from AE target FPS ranges, which is what Camera2
     * actually honours for a preview/record stream.
     */
    private fun supportedFrameRates(
        map: StreamConfigurationMap,
        characteristics: CameraCharacteristics,
    ): List<Int> {
        val ranges = characteristics.get(
            CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES,
        )
        val candidates = linkedSetOf<Int>()
        ranges?.forEach { range ->
            val upper = range.upper
            if (upper in 1..120) candidates += upper
            if (range.lower in 1..120) candidates += range.lower
        }

        // 30 and below come from the AE ranges, which is what a normal session
        // honours. Anything above that is only reachable in a high speed session,
        // and the AE ranges never list it: on this class of device they read
        // `[13,30] [30,30]` while the camera advertises `1280x720 30-120` for high
        // speed. So 60 is offered from the high speed configurations, not from AE.
        val interesting = listOf(24, 25, 30)
        val normal = interesting.filter { candidate -> candidates.any { it >= candidate } }

        val hz = map.getOutputMinFrameDuration(android.graphics.SurfaceTexture::class.java, Size(1280, 720))
        val capped = if (hz > 0) {
            val maxFps = (1_000_000_000L / hz).toInt()
            normal.filter { it <= maxFps }
        } else {
            normal
        }

        val highSpeedSixty = map.highSpeedVideoSizes?.any { size ->
            map.getHighSpeedVideoFpsRangesFor(size)?.any { it.upper >= 60 } == true
        } == true

        return (capped + if (highSpeedSixty) listOf(60) else emptyList())
            .distinct()
            .ifEmpty { listOf(30) }
    }

    private fun encoderSizesContains(encoderSizes: List<Pair<Int, Int>>, size: Pair<Int, Int>): Boolean {
        val (w, h) = size
        return encoderSizes.any { (ew, eh) ->
            (ew >= w && eh >= h) || (ew >= h && eh >= w)
        }
    }

    /** Whether the device exposes a hardware H.264 encoder at all. */
    fun hasHardwareH264Encoder(): Boolean = EncoderCapability.query().hardware

    companion object {
        /**
         * The camera list, probed once per process and shared.
         *
         * The list describes the phone's hardware and does not change while the
         * app runs, so every caller — the service, the session, the view model —
         * can be handed the same result instead of each walking CameraManager
         * and MediaCodecList for itself. The first caller runs the probe; the
         * rest wait for it and take the same list.
         */
        @Volatile
        private var shared: List<CameraInfo>? = null

        /** Returns the process-wide camera list, probing it on first use. */
        fun cameras(context: Context): List<CameraInfo> =
            shared ?: synchronized(this) {
                shared ?: CameraCapabilityProbe(context).probe().also { shared = it }
            }
    }
}

/**
 * The H.264 encoder's own capability limits.
 *
 * `hardware` drives the diagnostics panel (PRD §62). `supportedSizes` bounds the
 * resolutions the UI may offer so a size the encoder cannot take is never
 * presented as available.
 */
data class EncoderCapability(
    val hardware: Boolean,
    val supportedSizes: List<Pair<Int, Int>>,
    val maxBitrate: Int,
) {
    companion object {
        private const val TAG = "EncoderCapability"

        fun query(): EncoderCapability {
            val list = MediaCodecList(MediaCodecList.REGULAR_CODECS)
            val candidates = list.codecInfos.filter { info ->
                info.isEncoder && info.supportedTypes.any { it.equals(MediaFormat.MIMETYPE_VIDEO_AVC, true) }
            }
            if (candidates.isEmpty()) {
                Log.w(TAG, "no H.264 encoder reported by MediaCodecList")
                return EncoderCapability(hardware = false, supportedSizes = emptyList(), maxBitrate = 0)
            }

            // Prefer a hardware encoder; fall back to software so the product
            // still works on devices with only a software AVC encoder.
            val chosen = candidates.firstOrNull { info ->
                runCatching { info.isHardwareAccelerated }.getOrDefault(false)
            } ?: candidates.first()

            val caps = runCatching {
                chosen.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)
            }.getOrNull()

            val supported = buildList {
                val widths = caps?.videoCapabilities?.supportedWidths
                val heights = caps?.videoCapabilities?.supportedHeights
                if (widths != null && heights != null) {
                    // The intersection is expressed as a size-alignment check
                    // rather than a cross product, which would over-report.
                    val w = widths.upper
                    val h = heights.upper
                    if (w > 0 && h > 0) add(w to h)
                }
            }

            return EncoderCapability(
                hardware = runCatching { chosen.isHardwareAccelerated }.getOrDefault(false),
                supportedSizes = supported,
                maxBitrate = caps?.videoCapabilities?.bitrateRange?.upper ?: 0,
            )
        }
    }
}
