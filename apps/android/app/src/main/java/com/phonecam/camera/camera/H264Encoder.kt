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
    val buffer: ByteBuffer,
    val offset: Int,
    val length: Int,
    val ptsUs: Long,
    val isKeyframe: Boolean,
    val isCodecConfig: Boolean,
)

/**
 * Hardware H.264 encoder fed from a [Surface].
 *
 * The encoder renders into a Surface provided by CameraX, so frames never travel
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

    @Volatile
    private var running = false

    /**
     * The drain thread, held so [stop] can join it.
     *
     * The thread blocks in `dequeueOutputBuffer`, so it is unblocked by
     * `codec.stop()` rather than by a flag alone; joining after that is what
     * guarantees it has released its output buffers before the codec is torn
     * down, which `MediaCodec` requires and the previous fire-and-forget thread
     * did not guarantee.
     */
    @Volatile
    private var drainThread: Thread? = null

    /** The Surface to hand to CameraX as an output target. */
    val surface: Surface?
        get() = inputSurface

    val codecName: String?
        get() = runCatching { codec?.name }.getOrNull()

    fun start(): Surface? {
        if (running) return inputSurface
        return try {
            val format = MediaFormat.createVideoFormat(MIME, width, height).apply {
                setInteger(
                    MediaFormat.KEY_COLOR_FORMAT,
                    MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface,
                )
                setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
                setInteger(MediaFormat.KEY_FRAME_RATE, fps)
                setFloat(MediaFormat.KEY_I_FRAME_INTERVAL, I_FRAME_INTERVAL_SECONDS)
                // CBR, not VBR.
                //
                // With VBR a fast movement makes the encoder spend far more bits
                // for a second, and a webcam feels that twice: the frames get
                // larger exactly when the link and the decoder are busiest, so
                // the preview queue fills, whole groups of pictures are dropped
                // behind the movement, and the picture stairs and freezes. It
                // also lets the bitrate overshoot the budget the user chose. CBR
                // keeps each frame's cost predictable, which is what makes the
                // motion look even; quality follows the bitrate the user set
                // rather than spiking with the scene.
                setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR)
                setInteger(MediaFormat.KEY_PREPEND_HEADER_TO_SYNC_FRAMES, 1)
                // Ask the codec to run at the rate we asked for. Without an
                // operating rate some encoders pick a conservative default and
                // settle well below the requested frame rate — the picture is
                // sharp and the clock is fine, but the stream simply arrives at
                // 20 fps instead of 30. It is a hint, not a promise, so a codec
                // that ignores it behaves exactly as before.
                setInteger(MediaFormat.KEY_OPERATING_RATE, fps)
                // One frame of pipeline, not a buffer of them. A webcam cannot
                // use the quality a deep lookahead buys, and every buffered frame
                // is latency the viewer waits through.
                setInteger(MediaFormat.KEY_LATENCY, 1)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    // Lowest latency wins for a webcam; PRD §47 ranks latency
                    // above image quality.
                    setInteger(MediaFormat.KEY_PRIORITY, 0)
                    setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
                }
                setInteger(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            }

            val created = MediaCodec.createEncoderByType(MIME)
            created.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            val input = created.createInputSurface()
            created.start()

            codec = created
            inputSurface = input
            running = true
            startDrainLoop()
            Log.i(TAG, "encoder started ${width}x$height@$fps ${bitrate}bps mode=CBR name=${created.name}")
            input
        } catch (t: Throwable) {
            Log.e(TAG, "encoder start failed", t)
            onError("Couldn't start video encoding on this phone.")
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
        // `stop()` first: the drain thread is blocked in dequeueOutputBuffer with
        // an effectively infinite timeout, so a flag alone would not wake it.
        // Stopping the codec makes that call return, and the join then waits for
        // the thread to release its buffers before they are torn down.
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
     *
     * A dedicated thread keeps encoder latency independent of the camera
     * callback cadence, which is what holds end-to-end latency inside the
     * PRD §46 targets.
     *
     * The dequeue blocks until a frame is ready instead of polling every 10 ms.
     * `KEY_REPEAT_PREVIOUS_FRAME_AFTER` makes the codec emit a frame on its own
     * cadence, so the thread still wakes once per frame and a live stream gains
     * no latency; what it no longer does is wake 100 times a second between
     * frames. On a max-priority thread that idle wake-up was itself a small
     * amount of work on every frame, and the poll interval bounded the drain
     * rate from above — a blocking wait does neither. `codec.stop()` is what
     * unblocks the wait when the encoder is torn down.
     */
    private fun startDrainLoop() {
        drainThread = Thread({
            val target = codec ?: return@Thread
            while (running) {
                try {
                    val index = target.dequeueOutputBuffer(bufferInfo, DEQUEUE_TIMEOUT_US)
                    if (index == MediaCodec.INFO_TRY_AGAIN_LATER) continue
                    if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                        // Annex-B output means there is nothing to cache here; the
                        // SPS/PPS ride along with each keyframe.
                        continue
                    }
                    if (index < 0) continue

                    val buffer = target.getOutputBuffer(index)
                    if (buffer != null && bufferInfo.size > 0) {
                        val isConfig = bufferInfo.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                        val isKey = bufferInfo.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                        onFrame(
                            EncodedFrame(
                                buffer = buffer,
                                offset = bufferInfo.offset,
                                length = bufferInfo.size,
                                ptsUs = bufferInfo.presentationTimeUs,
                                isKeyframe = isKey || isConfig,
                                isCodecConfig = isConfig,
                            ),
                        )
                    }
                    target.releaseOutputBuffer(index, false)
                } catch (t: Throwable) {
                    // A codec stopped under a blocking dequeue throws; that is the
                    // normal teardown path, not an encoder failure.
                    if (!running) return@Thread
                    Log.e(TAG, "encoder drain failed", t)
                    onError("Video encoding stopped unexpectedly.")
                    running = false
                    return@Thread
                }
            }
        }, "phonecam-h264-encoder").apply {
            // Above normal, below the audio path: the drain loop must not be
            // starved by background work, but it no longer needs to out-rank
            // everything on the device to make a 10 ms poll deadline.
            priority = Thread.NORM_PRIORITY + 2
            start()
        }
    }

    companion object {
        private const val TAG = "H264Encoder"
        private const val MIME = MediaFormat.MIMETYPE_VIDEO_AVC
        private const val I_FRAME_INTERVAL_SECONDS = 0.5f

        /**
         * How long the drain loop waits for output before looping.
         *
         * Effectively infinite: the loop must not wake between frames, and the
         * only reason to leave the wait is that the encoder produced a frame or
         * that [stop] called `codec.stop()`. 100 ms is long enough to be free
         * when idle and short enough that a spurious wake (or a teardown that
         * races the flag) is noticed promptly.
         */
        private const val DEQUEUE_TIMEOUT_US = 100_000L

        /** Upper bound on waiting for the drain thread to finish after `codec.stop()`. */
        private const val DRAIN_JOIN_TIMEOUT_MS = 500L
    }
}
