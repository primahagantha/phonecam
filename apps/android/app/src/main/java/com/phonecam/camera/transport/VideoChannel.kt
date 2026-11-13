package com.phonecam.camera.transport

import android.util.Log
import com.phonecam.camera.camera.EncodedFrame
import com.phonecam.camera.protocol.Framing
import com.phonecam.camera.protocol.Proto
import com.phonecam.camera.protocol.VideoFlags
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/** Encoder-side counters the UI and the desktop stats panel both read. */
data class VideoChannelStats(
    val sentFrames: Long,
    val droppedFrames: Long,
    val bytesSent: Long,
) {
    val dropPercent: Double
        get() {
            val total = sentFrames + droppedFrames
            return if (total == 0L) 0.0 else droppedFrames.toDouble() * 100.0 / total
        }
}

/**
 * Writes encoded H.264 to the desktop's video channel.
 *
 * Encoding must never block on the network: if the socket falls behind, frames
 * are dropped and counted instead of stalling the encoder, because PRD §47 ranks
 * stability above completeness and a stalled encoder degrades the whole stream.
 * Dropped frames feed the adaptive quality controller (PRD §48).
 */
class VideoChannel(
    private val connection: PeerConnection,
    private val onClosed: ((Throwable?) -> Unit)? = null,
) {

    private val queue = ArrayBlockingQueue<PendingFrame>(QUEUE_CAPACITY)
    private val recyclePool = ArrayBlockingQueue<PendingFrame>(POOL_CAPACITY)
    private val running = AtomicBoolean(false)
    private val sentFrames = AtomicLong(0)
    private val droppedFrames = AtomicLong(0)
    private val bytesSent = AtomicLong(0)

    /** Reporting state for `reportIfDue`; see that function for why it exists. */
    private val largestFrameBytes = AtomicLong(0)
    private val lastReportAtMs = AtomicLong(0)
    private var lastSent = 0L
    private var lastDropped = 0L
    private var lastBytes = 0L

    private var writerThread: Thread? = null

    private class PendingFrame(
        var data: ByteArray,
        var length: Int = 0,
        var ptsUs: Long = 0L,
        var keyframe: Boolean = false,
        var codecConfig: Boolean = false,
    )

    fun start() {
        if (!running.compareAndSet(false, true)) return
        writerThread = Thread({ writeLoop() }, "phonecam-video-writer").apply {
            priority = Thread.MAX_PRIORITY
            start()
        }
    }

    fun stop() {
        running.set(false)
        writerThread?.interrupt()
        writerThread = null
        queue.clear()
        recyclePool.clear()
        runCatching { connection.close() }
    }

    val isRunning: Boolean
        get() = running.get()

    val stats: VideoChannelStats
        get() = VideoChannelStats(sentFrames.get(), droppedFrames.get(), bytesSent.get())

    private fun acquireFrame(neededLength: Int): PendingFrame {
        val reused = recyclePool.poll()
        if (reused != null) {
            if (reused.data.size < neededLength) {
                reused.data = ByteArray(maxOf(neededLength, MIN_POOLED_FRAME_BYTES))
            }
            return reused
        }
        return PendingFrame(ByteArray(maxOf(neededLength, MIN_POOLED_FRAME_BYTES)))
    }

    private fun recycleFrame(frame: PendingFrame) {
        if (frame.data.size <= MAX_POOLED_FRAME_BYTES) {
            frame.length = 0
            recyclePool.offer(frame)
        }
    }

    /**
     * Copies [frame] out of the MediaCodec buffer and enqueues it.
     *
     * The copy is unavoidable: the encoder's buffer is recycled as soon as the
     * callback returns, and the write happens on another thread. Frame backing
     * arrays are pooled to eliminate steady-state GC churn, and the queue is
     * short and bounded so a slow link drops rather than accumulates latency.
     */
    fun submit(frame: EncodedFrame) {
        if (!running.get() || frame.length <= 0) return

        val pending = acquireFrame(frame.length)
        val buf = frame.buffer
        if (buf.hasArray()) {
            System.arraycopy(buf.array(), buf.arrayOffset() + frame.offset, pending.data, 0, frame.length)
        } else {
            val prevPos = buf.position()
            try {
                buf.position(frame.offset)
                buf.get(pending.data, 0, frame.length)
            } finally {
                buf.position(prevPos)
            }
        }
        pending.length = frame.length
        pending.ptsUs = frame.ptsUs
        pending.keyframe = frame.isKeyframe
        pending.codecConfig = frame.isCodecConfig

        if (!queue.offer(pending)) {
            // Prefer dropping a non-keyframe; a dropped keyframe costs the peer a
            // whole GOP, so replace the queued head when this frame is a keyframe.
            if (pending.keyframe) {
                val evicted = queue.poll()
                if (evicted != null) {
                    droppedFrames.incrementAndGet()
                    recycleFrame(evicted)
                }
                if (!queue.offer(pending)) {
                    droppedFrames.incrementAndGet()
                    recycleFrame(pending)
                }
            } else {
                droppedFrames.incrementAndGet()
                recycleFrame(pending)
            }
        }
    }

    private fun writeLoop() {
        var frameId = 0L
        val headerScratch = ByteArray(Proto.VIDEO_FRAME_PREFIX_SIZE)
        var unflushed = 0
        while (running.get()) {
            val pending = try {
                queue.take()
            } catch (interrupted: InterruptedException) {
                if (!running.get()) return
                continue
            }

            try {
                var flags = VideoFlags.NONE
                if (pending.keyframe) flags = flags or VideoFlags.KEYFRAME
                if (pending.codecConfig) flags = flags or VideoFlags.CODEC_CONFIG

                val length = pending.length
                unflushed++
                val shouldFlush = queue.isEmpty() || unflushed >= FLUSH_BATCH_LIMIT
                Framing.writeVideoPacket(
                    output = connection.output,
                    frameId = frameId++,
                    ptsUs = pending.ptsUs,
                    flags = flags,
                    codec = CODEC_H264,
                    payload = pending.data,
                    payloadOffset = 0,
                    payloadLength = length,
                    headerScratch = headerScratch,
                    flush = shouldFlush,
                )
                if (shouldFlush) unflushed = 0
                recycleFrame(pending)
                sentFrames.incrementAndGet()
                bytesSent.addAndGet(length.toLong())
                // Largest frame in this reporting window. A sudden rise is the
                // signature of fast movement and it is the number that predicts
                // stutter: big frames fill the queue and the link at once.
                largestFrameBytes.set(
                    maxOf(largestFrameBytes.get(), length.toLong()),
                )
                reportIfDue()
            } catch (t: Throwable) {
                if (running.get()) {
                    Log.w(TAG, "video write failed: ${t.javaClass.simpleName}")
                }
                running.set(false)
                runCatching { connection.close() }
                onClosed?.invoke(t)
                return
            }
        }
    }

    /**
     * One line per second describing whether the picture is actually getting
     * out.
     *
     * "It froze" is a report the logs could not previously confirm or deny: the
     * counters existed but nothing ever printed them together, so a stream that
     * was dropping every other frame looked identical to one that was healthy.
     * The line carries the three numbers that separate them — sent, dropped and
     * queued — plus the largest frame, because a spike there is movement
     * outrunning the bitrate.
     */
    private fun reportIfDue() {
        val now = System.currentTimeMillis()
        val last = lastReportAtMs.get()
        if (last != 0L && now - last < REPORT_INTERVAL_MS) return
        if (!lastReportAtMs.compareAndSet(last, now)) return

        val sent = sentFrames.get()
        val dropped = droppedFrames.get()
        val bytes = bytesSent.get()
        val largest = largestFrameBytes.getAndSet(0)
        val queued = queue.size
        val kbps = if (last == 0L) 0L else ((bytes - lastBytes) * 8_000L) / (now - last).coerceAtLeast(1L)
        lastBytes = bytes
        Log.i(
            TAG,
            "video out: ${sent - lastSent} fps, dropped=$dropped (+${dropped - lastDropped}), " +
                "queued=$queued, ${kbps / 1000}kbps, largest=${largest / 1024}kB",
        )
        lastSent = sent
        lastDropped = dropped
    }

    private companion object {
        const val TAG = "VideoChannel"
        const val CODEC_H264 = 1

        /** How often `reportIfDue` writes its one-line summary. */
        const val REPORT_INTERVAL_MS = 1_000L

        /**
         * Deep enough to absorb a scheduling hiccup, shallow enough that a
         * congested link shows up as dropped frames rather than growing latency.
         */
        const val QUEUE_CAPACITY = 15
        const val POOL_CAPACITY = QUEUE_CAPACITY + 2
        const val MIN_POOLED_FRAME_BYTES = 64 * 1024
        const val MAX_POOLED_FRAME_BYTES = 512 * 1024
        const val FLUSH_BATCH_LIMIT = 4
    }
}
