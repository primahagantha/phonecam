package com.phonecam.camera.transport

import com.phonecam.camera.camera.EncodedFrame
import com.phonecam.camera.protocol.ChannelRole
import com.phonecam.camera.protocol.HandshakeStatus
import com.phonecam.camera.protocol.ServerResponse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.IOException
import java.io.OutputStream
import java.net.Socket
import java.nio.ByteBuffer
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

class VideoChannelTest {

    private fun testConnection(out: OutputStream): PeerConnection = PeerConnection(
        socket = Socket(),
        input = DataInputStream(ByteArrayInputStream(ByteArray(0))),
        output = DataOutputStream(out),
        role = ChannelRole.VIDEO,
        response = ServerResponse(1, HandshakeStatus.OK, ChannelRole.VIDEO.wire, ""),
    )

    @Test
    fun `starts and stops cleanly`() {
        val out = ByteArrayOutputStream()
        val channel = VideoChannel(testConnection(out))
        assertFalse(channel.isRunning)

        channel.start()
        assertTrue(channel.isRunning)

        channel.stop()
        assertFalse(channel.isRunning)
    }

    @Test
    fun `broken socket notifies onClosed callback and stops running`() {
        val latch = CountDownLatch(1)
        val caughtError = AtomicReference<Throwable?>(null)
        val failingOut = object : OutputStream() {
            override fun write(b: Int) {
                throw IOException("Broken pipe")
            }
            override fun write(b: ByteArray, off: Int, len: Int) {
                throw IOException("Broken pipe")
            }
        }

        val channel = VideoChannel(
            connection = testConnection(failingOut),
            onClosed = { error ->
                caughtError.set(error)
                latch.countDown()
            },
        )

        channel.start()
        assertTrue(channel.isRunning)

        val frameData = ByteArray(100) { 1 }
        val buffer = ByteBuffer.wrap(frameData)
        val frame = EncodedFrame(
            buffer = buffer,
            offset = 0,
            length = 100,
            ptsUs = 1000L,
            isKeyframe = true,
            isCodecConfig = false,
        )

        channel.submit(frame)

        val notified = latch.await(2, TimeUnit.SECONDS)
        assertTrue("onClosed must be called when socket write fails", notified)
        assertNotNull(caughtError.get())
        assertTrue(caughtError.get() is IOException)
        assertFalse(channel.isRunning)
    }

    @Test
    fun `clean stop does not notify onClosed callback`() {
        val onClosedCalled = AtomicBoolean(false)
        val out = ByteArrayOutputStream()
        val channel = VideoChannel(
            connection = testConnection(out),
            onClosed = { onClosedCalled.set(true) },
        )
        channel.start()
        assertTrue(channel.isRunning)

        channel.stop()
        assertFalse(channel.isRunning)
        Thread.sleep(100)
        assertFalse("onClosed must not be called on clean stop", onClosedCalled.get())
    }

    @Test
    fun `eviction on keyframe preserves codec config frames`() {
        val writeBlocker = CountDownLatch(1)
        val writtenBytes = ByteArrayOutputStream()
        val blockingOut = object : OutputStream() {
            private var first = true
            override fun write(b: Int) {
                if (first) {
                    first = false
                    writeBlocker.await(5, TimeUnit.SECONDS)
                }
                writtenBytes.write(b)
            }
            override fun write(b: ByteArray, off: Int, len: Int) {
                if (first) {
                    first = false
                    writeBlocker.await(5, TimeUnit.SECONDS)
                }
                writtenBytes.write(b, off, len)
            }
        }

        val channel = VideoChannel(testConnection(blockingOut))
        channel.start()

        fun makeFrame(isKey: Boolean, isConfig: Boolean): EncodedFrame {
            val data = ByteArray(32) { 0x42 }
            return EncodedFrame(
                buffer = ByteBuffer.wrap(data),
                offset = 0,
                length = data.size,
                ptsUs = 1000L,
                isKeyframe = isKey,
                isCodecConfig = isConfig,
            )
        }

        // Submit first frame which blocks writer thread in write()
        channel.submit(makeFrame(isKey = false, isConfig = false))
        Thread.sleep(50)

        // Fill queue with 3 delta frames and 1 codec config frame
        channel.submit(makeFrame(isKey = false, isConfig = false))
        channel.submit(makeFrame(isKey = false, isConfig = true)) // SPS/PPS
        channel.submit(makeFrame(isKey = false, isConfig = false))

        // Now submit IDR keyframe; this triggers catch-up eviction (queue.size >= 3)
        channel.submit(makeFrame(isKey = true, isConfig = false))

        // Unblock writer
        writeBlocker.countDown()
        Thread.sleep(100)
        channel.stop()

        // 2 delta frames were evicted, but the codec config frame was preserved
        assertEquals(2L, channel.stats.droppedFrames)
    }
}
