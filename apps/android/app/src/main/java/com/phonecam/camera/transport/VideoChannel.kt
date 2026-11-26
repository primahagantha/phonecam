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
    private val onRequestKeyframe: (() -> Unit)? = null,
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
        // Enforce low-latency TCP socket tuning even if connection was supplied directly.
        runCatching { connection.socket.tcpNoDelay = true }
        runCatching { connection.socket.sendBufferSize = VIDEO_SEND_BUFFER_BYTES }
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

        // Low-latency catch-up: if queue has accumulated lag when a fresh IDR keyframe
        // arrives, all queued frames belong to the obsolete previous GOP. Evict them
        // so the stream catches up instantly to real-time, but keep any frame where
        // codecConfig == true so SPS/PPS is not dropped.
        if (pending.keyframe && queue.size >= KEYFRAME_EVICT_THRESHOLD) {
            evictStaleFramesOnKeyframe()
        }

        if (!queue.offer(pending)) {
            // Prefer dropping non-keyframes; a keyframe or codecConfig must replace stale frames.
            if (pending.keyframe || pending.codecConfig) {
                evictStaleFramesOnKeyframe()
                if (!queue.offer(pending)) {
                    droppedFrames.incrementAndGet()
                    recycleFrame(pending)
                    onRequestKeyframe?.invoke()
                }
            } else {
                droppedFrames.incrementAndGet()
                recycleFrame(pending)
                onRequestKeyframe?.invoke()
            }
        }
    }

    private fun evictStaleFramesOnKeyframe() {
        val toKeep = mutableListOf<PendingFrame>()
        var evictedCount = 0
        while (!queue.isEmpty()) {
            val evicted = queue.poll() ?: break
            if (evicted.codecConfig) {
                toKeep.add(evicted)
            } else {
                droppedFrames.incrementAndGet()
                recycleFrame(evicted)
                evictedCount++
            }
        }
        for (saved in toKeep) {
            queue.offer(saved)
        }
        if (evictedCount > 0) {
            onRequestKeyframe?.invoke()
        }
    }

    private fun writeLoop() {
        var frameId = 0L
        val headerScratch = ByteArray(Proto.VIDEO_FRAME_PREFIX_SIZE)
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
                    flush = true,
                )
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
                val wasRunning = running.get()
                if (wasRunning) {
                    Log.w(TAG, "video write failed: ${t.javaClass.simpleName}")
                }
                running.set(false)
                runCatching { connection.close() }
                if (wasRunning) {
                    onClosed?.invoke(t)
                }
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

    internal companion object {
        const val TAG = "VideoChannel"
        const val CODEC_H264 = 1

        /** How often `reportIfDue` writes its one-line summary. */
        const val REPORT_INTERVAL_MS = 1_000L

        /**
         * Tight 3-frame ceiling keeps end-to-end queuing delay under ~50ms at 60 FPS
         * (~100ms at 30 FPS) while still absorbing single-frame scheduler jitter.
         */
        const val QUEUE_CAPACITY = 3
        const val KEYFRAME_EVICT_THRESHOLD = 2
        const val VIDEO_SEND_BUFFER_BYTES = 128 * 1024
        const val POOL_CAPACITY = QUEUE_CAPACITY + 2
        const val MIN_POOLED_FRAME_BYTES = 192 * 1024
        const val MAX_POOLED_FRAME_BYTES = 512 * 1024
    }
}
