package com.phonecam.camera.camera

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.Surface
import java.nio.ByteBuffer

/** One encoded H.264 access unit, still in Annex-B form. */
class EncodedFrame(
    var buffer: ByteBuffer,
    var offset: Int,
    var length: Int,
    var ptsUs: Long,
    var isKeyframe: Boolean,
    var isCodecConfig: Boolean,
)

/**
 * Hardware / resilient software H.264 encoder fed from a [Surface].
 *
 * The encoder renders into a Surface provided by Camera2, so frames never travel
 * through the app's heap as raw pixels — the encoder's own buffer queue is the
 * only place pixel data lives (PRD §12: never send raw frames).
 *
 * Output is Annex-B. On API 26+ MediaCodec emits Annex-B for the AVC encoder by
 * default, and `KEY_PREPEND_HEADER_TO_SYNC_FRAMES` makes every keyframe carry
 * SPS+PPS, which is what makes mid-stream decoder (re)attachment work without a
 * separate configuration exchange.
 */
class H264Encoder(
    private val width: Int,
    private val height: Int,
    private val fps: Int,
    private val bitrate: Int,
    private val onFrame: (EncodedFrame) -> Unit,
    private val onError: (String) -> Unit,
) {

    private var codec: MediaCodec? = null
    private var inputSurface: Surface? = null
    private val bufferInfo = MediaCodec.BufferInfo()

    var configuredWidth: Int = width
        private set
    var configuredHeight: Int = height
        private set
    var configuredFps: Int = fps
        private set
    var configuredBitrate: Int = bitrate
        private set
    var configuredCodecName: String = "unknown"
        private set
    var isSubstituted: Boolean = false
        private set

    @Volatile
    private var running = false

    @Volatile
    private var drainThread: Thread? = null

    /** The Surface to hand to Camera2 as an output target. */
    val surface: Surface?
        get() = inputSurface

    val codecName: String?
        get() = runCatching { codec?.name }.getOrNull() ?: configuredCodecName

    private data class EncodeCandidate(
        val width: Int,
        val height: Int,
        val fps: Int,
        val bitrate: Int,
    )

    fun start(): Surface? {
        if (running) return inputSurface

        Log.i(TAG, "requesting encoder: ${width}x${height} @ ${fps}fps, bitrate=$bitrate")

        val candidates = buildList {
            add(EncodeCandidate(width, height, fps, bitrate))
            if (width > 1280 || height > 720) {
                add(EncodeCandidate(1280, 720, fps.coerceAtMost(30), bitrate.coerceAtMost(4_000_000)))
            }
            if (width > 854 || height > 480) {
                add(EncodeCandidate(854, 480, fps.coerceAtMost(30), bitrate.coerceAtMost(2_000_000)))
            }
            if (width > 640 || height > 480) {
                add(EncodeCandidate(640, 480, fps.coerceAtMost(30), bitrate.coerceAtMost(1_500_000)))
            }
        }.distinctBy { it.width to it.height }

        for ((index, cand) in candidates.withIndex()) {
            val isFallback = index > 0
            // First try default/hardware encoder, then software fallback
            val codecAttempts = listOf<String?>(null, "c2.android.avc.encoder", "OMX.google.h264.encoder")

            for (codecName in codecAttempts) {
                val surface = tryConfigureAndStart(cand, codecName, isFallback)
                if (surface != null) {
                    configuredWidth = cand.width
                    configuredHeight = cand.height
                    configuredFps = cand.fps
                    configuredBitrate = cand.bitrate
                    isSubstituted = isFallback
                    Log.i(
                        TAG,
                        "encoder successfully active: ${configuredWidth}x${configuredHeight}@${configuredFps} " +
                            "(${configuredCodecName}, substituted=$isSubstituted)"
                    )
                    return surface
                }
            }
        }

        Log.e(TAG, "all encoder candidates failed")
        onError("Couldn't start video encoding on this phone.")
        return null
    }

    private fun tryConfigureAndStart(
        cand: EncodeCandidate,
        specificCodecName: String?,
        isFallback: Boolean,
    ): Surface? {
        var created: MediaCodec? = null
        var surface: Surface? = null

        return try {
            created = if (specificCodecName != null) {
                runCatching { MediaCodec.createByCodecName(specificCodecName) }.getOrNull() ?: return null
            } else {
                runCatching { MediaCodec.createEncoderByType(MIME) }.getOrNull() ?: return null
            }

            val caps = runCatching { created.codecInfo.getCapabilitiesForType(MIME) }.getOrNull()
            val isCbr = caps?.encoderCapabilities?.isBitrateModeSupported(
                MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR
            ) == true

            val format = MediaFormat.createVideoFormat(MIME, cand.width, cand.height).apply {
                setInteger(
                    MediaFormat.KEY_COLOR_FORMAT,
                    MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface,
                )
                setInteger(MediaFormat.KEY_BIT_RATE, cand.bitrate)
                setInteger(MediaFormat.KEY_FRAME_RATE, cand.fps)
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, I_FRAME_INTERVAL_SECONDS)
                if (isCbr && specificCodecName == null) {
                    setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR)
                }
            }

            var configured = false
            try {
                created.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                configured = true
            } catch (e: Throwable) {
                Log.w(TAG, "configure failed on ${created.name} (${cand.width}x${cand.height}): ${e.message}")
                runCatching { created.release() }
                created = null
            }

            if (!configured) {
                // Try a clean fallback instance with simplest profile
                created = if (specificCodecName != null) {
                    runCatching { MediaCodec.createByCodecName(specificCodecName) }.getOrNull() ?: return null
                } else {
                    runCatching { MediaCodec.createEncoderByType(MIME) }.getOrNull() ?: return null
                }

                val basicFormat = MediaFormat.createVideoFormat(MIME, cand.width, cand.height).apply {
                    setInteger(
                        MediaFormat.KEY_COLOR_FORMAT,
                        MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface,
                    )
                    setInteger(MediaFormat.KEY_BIT_RATE, cand.bitrate)
                    setInteger(MediaFormat.KEY_FRAME_RATE, cand.fps)
                    setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
                }
                try {
                    created.configure(basicFormat, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                } catch (e: Throwable) {
                    Log.w(TAG, "basic configure failed on ${created.name}: ${e.message}")
                    runCatching { created.release() }
                    return null
                }
            }

            val activeCodec = created ?: return null
            surface = activeCodec.createInputSurface()
            activeCodec.start()

            codec = activeCodec
            inputSurface = surface
            configuredCodecName = activeCodec.name
            running = true
            startDrainLoop()
            surface
        } catch (t: Throwable) {
            runCatching { created?.stop() }
            runCatching { created?.release() }
            runCatching { surface?.release() }
            null
        }
    }

    /** Live bitrate change without restarting the encoder (PRD §48). */
    fun updateBitrate(newBitrate: Int) {
        val target = codec ?: return
        runCatching {
            val params = Bundle().apply {
                putInt(MediaCodec.PARAMETER_KEY_VIDEO_BITRATE, newBitrate)
            }
            target.setParameters(params)
        }.onFailure { Log.w(TAG, "bitrate update rejected: ${it.javaClass.simpleName}") }
    }

    /** Forces a keyframe so a newly attached decoder can start immediately. */
    fun requestKeyframe() {
        val target = codec ?: return
        runCatching {
            target.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
        }
    }

    fun stop() {
        running = false
        runCatching { codec?.stop() }
        runCatching { drainThread?.join(DRAIN_JOIN_TIMEOUT_MS) }
        drainThread = null
        runCatching { codec?.release() }
        runCatching { inputSurface?.release() }
        codec = null
        inputSurface = null
    }

    /**
     * Drains the encoder on its own thread.
     */
    private fun startDrainLoop() {
        drainThread = Thread({
            val target = codec ?: return@Thread
            var reusableFrame: EncodedFrame? = null
            var cachedConfig: ByteArray? = null
            while (running) {
                try {
                    val index = target.dequeueOutputBuffer(bufferInfo, DEQUEUE_TIMEOUT_US)
                    if (index == MediaCodec.INFO_TRY_AGAIN_LATER) continue
                    if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                        continue
                    }
                    if (index < 0) continue

                    try {
                        val buffer = target.getOutputBuffer(index)
                        if (buffer != null && bufferInfo.size > 0) {
                            val isConfig = bufferInfo.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                            val isKey = bufferInfo.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0

                            if (isConfig) {
                                val configBytes = ByteArray(bufferInfo.size)
                                val pos = buffer.position()
                                try {
                                    buffer.position(bufferInfo.offset)
                                    buffer.get(configBytes)
                                } finally {
                                    buffer.position(pos)
                                }
                                cachedConfig = configBytes
                            }

                            val outputBuf: ByteBuffer
                            val outputOffset: Int
                            val outputLength: Int
                            val cfg = cachedConfig
                            if (isKey && !isConfig && cfg != null && !hasAnnexBSps(buffer, bufferInfo.offset, bufferInfo.size)) {
                                val combined = ByteArray(cfg.size + bufferInfo.size)
                                System.arraycopy(cfg, 0, combined, 0, cfg.size)
                                val pos = buffer.position()
                                try {
                                    buffer.position(bufferInfo.offset)
                                    buffer.get(combined, cfg.size, bufferInfo.size)
                                } finally {
                                    buffer.position(pos)
                                }
                                outputBuf = ByteBuffer.wrap(combined)
                                outputOffset = 0
                                outputLength = combined.size
                            } else {
                                outputBuf = buffer
                                outputOffset = bufferInfo.offset
                                outputLength = bufferInfo.size
                            }

                            val frame = reusableFrame?.apply {
                                this.buffer = outputBuf
                                this.offset = outputOffset
                                this.length = outputLength
                                this.ptsUs = bufferInfo.presentationTimeUs
                                this.isKeyframe = isKey || isConfig
                                this.isCodecConfig = isConfig
                            } ?: EncodedFrame(
                                buffer = outputBuf,
                                offset = outputOffset,
                                length = outputLength,
                                ptsUs = bufferInfo.presentationTimeUs,
                                isKeyframe = isKey || isConfig,
                                isCodecConfig = isConfig,
                            ).also { reusableFrame = it }
                            onFrame(frame)
                        }
                    } finally {
                        target.releaseOutputBuffer(index, false)
                    }
                } catch (t: Throwable) {
                    if (!running) return@Thread
                    Log.e(TAG, "encoder drain failed", t)
                    onError("Video encoding stopped unexpectedly.")
                    running = false
                    return@Thread
                }
            }
        }, "phonecam-h264-encoder").apply {
            priority = Thread.NORM_PRIORITY + 2
            start()
        }
    }

    companion object {
        private const val TAG = "H264Encoder"
        private const val MIME = MediaFormat.MIMETYPE_VIDEO_AVC
        private const val I_FRAME_INTERVAL_SECONDS = 1
        private const val DEQUEUE_TIMEOUT_US = 100_000L
        private const val DRAIN_JOIN_TIMEOUT_MS = 500L

        private fun hasAnnexBSps(buffer: ByteBuffer, offset: Int, size: Int): Boolean {
            if (size < 5) return false
            val pos = buffer.position()
            return try {
                buffer.position(offset)
                val b0 = buffer.get()
                val b1 = buffer.get()
                val b2 = buffer.get()
                if (b0 == 0.toByte() && b1 == 0.toByte() && b2 == 0.toByte() && size >= 5) {
                    val b3 = buffer.get()
                    val b4 = buffer.get()
                    b3 == 1.toByte() && (b4.toInt() and 0x1F) == 7
                } else if (b0 == 0.toByte() && b1 == 0.toByte() && b2 == 1.toByte()) {
                    val b3 = buffer.get()
                    (b3.toInt() and 0x1F) == 7
                } else {
                    false
                }
            } finally {
                buffer.position(pos)
            }
        }
    }
}
