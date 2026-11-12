package com.phonecam.camera.session

import com.phonecam.camera.nativecore.NativeCore
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.io.Closeable

@Serializable
data class AdaptiveQualityDecision(
    val action: String,
    val resolution: String,
    val fps: Int,
    val bitrate: Int,
    val label: String,
    val reason: String,
) {
    val isChange: Boolean
        get() = action == "step_down" || action == "step_up"
}

/**
 * Lifecycle-safe wrapper around the Rust `AdaptiveQuality` controller via [NativeCore].
 *
 * Thread-safe on the native handle: [close] frees the boxed Rust instance at most
 * once, and subsequent calls become safe no-ops / `"hold"` decisions.
 */
class NativeAdaptiveQuality(
    enabled: Boolean = true,
    initResolution: String = "1920x1080",
    initFps: Int = 30,
) : Closeable {

    private val lock = Any()

    @Volatile
    private var handle: Long = NativeCore.adaptiveQualityCreate(enabled, initResolution, initFps)

    val isOpen: Boolean
        get() = handle != 0L

    fun setEnabled(enabled: Boolean) {
        synchronized(lock) {
            val h = handle
            if (h != 0L) {
                NativeCore.adaptiveQualitySetEnabled(h, enabled)
            }
        }
    }

    fun confirm(resolution: String, fps: Int) {
        synchronized(lock) {
            val h = handle
            if (h != 0L) {
                NativeCore.adaptiveQualityConfirm(h, resolution, fps)
            }
        }
    }

    fun observe(
        nowMs: Long,
        currentResolution: String,
        currentFps: Int,
        maxResolution: String,
        maxFps: Int,
        droppedPercent: Double,
        rttUs: Long = 0L,
        thermal: String = "none",
        decodeStruggling: Boolean = false,
    ): AdaptiveQualityDecision {
        val raw = synchronized(lock) {
            val h = handle
            if (h == 0L) {
                return AdaptiveQualityDecision(
                    action = "hold",
                    resolution = currentResolution,
                    fps = currentFps,
                    bitrate = 0,
                    label = "",
                    reason = "",
                )
            }
            NativeCore.adaptiveQualityObserve(
                handle = h,
                nowMs = nowMs,
                curRes = currentResolution,
                curFps = currentFps,
                maxRes = maxResolution,
                maxFps = maxFps,
                droppedPct = droppedPercent,
                rttUs = rttUs,
                thermal = thermal,
                decodeStruggling = decodeStruggling,
            )
        }
        return json.decodeFromString(AdaptiveQualityDecision.serializer(), raw)
    }

    override fun close() {
        synchronized(lock) {
            val h = handle
            if (h != 0L) {
                handle = 0L
                NativeCore.adaptiveQualityDestroy(h)
            }
        }
    }

    private companion object {
        val json = Json { ignoreUnknownKeys = true }
    }
}
