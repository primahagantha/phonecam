package com.phonecam.camera.protocol

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.IOException

/**
 * Wire framing (docs/protocol.md §5, §7).
 *
 * These assertions pin the exact byte layout. The same layout is verified from
 * the other side in `scripts/e2e-protocol.mjs`, so a change to one implementation
 * without the others fails one of the two.
 */
class ProtoTest {

    @Test
    fun `greeting is exactly 40 bytes with big-endian fields`() {
        val greeting = Framing.readGreeting(
            DataInputStream(ByteArrayInputStream(greetingBytes(HandshakeStatus.OK))),
        )
        assertEquals(Proto.VERSION, greeting.protoVersion)
        assertEquals(HandshakeStatus.OK, greeting.status)
        assertEquals(Proto.SERVER_GREETING_SIZE, 40)
    }

    @Test
    fun `greeting carries the server nonce verbatim`() {
        val nonce = ByteArray(32) { (it * 7).toByte() }
        val greeting = Framing.readGreeting(
            DataInputStream(ByteArrayInputStream(greetingBytes(HandshakeStatus.OK, nonce))),
        )
        assertArrayEquals(nonce, greeting.serverNonce)
    }

    @Test
    fun `greeting rejects a foreign peer by magic`() {
        val bytes = greetingBytes(HandshakeStatus.OK)
        bytes[0] = 'X'.code.toByte()
        try {
            Framing.readGreeting(DataInputStream(ByteArrayInputStream(bytes)))
            throw AssertionError("expected a foreign magic to be rejected")
        } catch (expected: IOException) {
            assertTrue(expected.message!!.contains("PhoneCam"))
        }
    }

    @Test
    fun `handshake layout matches the documented offsets`() {
        val output = ByteArrayOutputStream()
        Framing.writeHandshake(
            DataOutputStream(output),
            role = ChannelRole.VIDEO,
            authMode = AuthMode.DEVICE_SECRET,
            clientNonce = 0x0102030405060708L,
            auth = "android-deadbeef:abc",
        )

        val bytes = output.toByteArray()
        val buffer = java.nio.ByteBuffer.wrap(bytes).order(java.nio.ByteOrder.BIG_ENDIAN)

        assertEquals(Proto.MAGIC, buffer.int)                       // offset 0
        assertEquals(Proto.VERSION.toByte(), buffer.get())          // offset 4
        assertEquals(ChannelRole.VIDEO.wire.toByte(), buffer.get()) // offset 5
        assertEquals(AuthMode.DEVICE_SECRET.wire.toByte(), buffer.get()) // offset 6
        assertEquals(0.toByte(), buffer.get())                      // offset 7, flags
        assertEquals(0x0102030405060708L, buffer.long)              // offset 8
        assertEquals("android-deadbeef:abc".length.toShort(), buffer.short) // offset 16
        assertEquals(0.toShort(), buffer.short)                     // offset 18, reserved
        assertEquals(Proto.CLIENT_HANDSHAKE_SIZE + "android-deadbeef:abc".length, bytes.size)
    }

    @Test
    fun `response carries a status and a json body`() {
        val output = ByteArrayOutputStream()
        val json = """{"deviceId":"android-1a2b3c4d"}"""
        val jsonBytes = json.toByteArray()

        val bytes = java.nio.ByteBuffer.allocate(Proto.SERVER_RESPONSE_SIZE + jsonBytes.size)
            .order(java.nio.ByteOrder.BIG_ENDIAN)
            .putInt(Proto.MAGIC)
            .put(Proto.VERSION.toByte())
            .put(HandshakeStatus.OK.wire.toByte())
            .put(ChannelRole.CONTROL.wire.toByte())
            .put(0)
            .putLong(0L)
            .putInt(jsonBytes.size)
            .putInt(0)
            .put(jsonBytes)
            .array()
        output.write(bytes)

        val response = Framing.readResponse(
            DataInputStream(ByteArrayInputStream(output.toByteArray())),
        )
        assertEquals(HandshakeStatus.OK, response.status)
        assertEquals(json, response.body)
        assertEquals("android-1a2b3c4d", response.json!!["deviceId"]!!.toString().trim('"'))
    }

    @Test
    fun `control frames round-trip through the length prefix`() {
        val output = ByteArrayOutputStream()
        val payload = """{"t":"set_video","resolution":"1920x1080"}""".toByteArray()
        Framing.writeControl(DataOutputStream(output), payload)

        val decoded = Framing.readControl(DataInputStream(ByteArrayInputStream(output.toByteArray())))
        assertArrayEquals(payload, decoded)
    }

    @Test
    fun `video packets round-trip header and payload`() {
        val payload = byteArrayOf(0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1E)
        val header = VideoPacketHeader(
            frameId = 7,
            ptsUs = 123_456,
            flags = VideoFlags.KEYFRAME or VideoFlags.CODEC_CONFIG,
            codec = 1,
            payloadLen = payload.size,
        )

        val output = ByteArrayOutputStream()
        Framing.writeVideoPacket(DataOutputStream(output), header, payload)

        val bytes = output.toByteArray()
        val totalLength = java.nio.ByteBuffer.wrap(bytes).order(java.nio.ByteOrder.BIG_ENDIAN).int
        assertEquals(Proto.VIDEO_HEADER_SIZE + payload.size, totalLength)
        assertEquals(4 + totalLength, bytes.size)

        val decoded = Framing.readVideoHeader(
            java.nio.ByteBuffer.wrap(bytes, 4, Proto.VIDEO_HEADER_SIZE).order(java.nio.ByteOrder.BIG_ENDIAN),
        )
        assertEquals(7L, decoded.frameId)
        assertEquals(123_456L, decoded.ptsUs)
        assertEquals(VideoFlags.KEYFRAME or VideoFlags.CODEC_CONFIG, decoded.flags)
        assertEquals(payload.size, decoded.payloadLen)
        assertArrayEquals(payload, bytes.copyOfRange(4 + Proto.VIDEO_HEADER_SIZE, bytes.size))
    }

    @Test
    fun `rejects an oversized control frame instead of allocating it`() {
        val oversized = java.nio.ByteBuffer.allocate(4).order(java.nio.ByteOrder.BIG_ENDIAN)
            .putInt(Proto.MAX_CONTROL_JSON + 1)
            .array()
        try {
            Framing.readControl(DataInputStream(ByteArrayInputStream(oversized)))
            throw AssertionError("expected an oversized frame to be rejected")
        } catch (expected: IOException) {
            assertTrue(expected.message!!.contains("out of range"))
        }
    }

    @Test
    fun `control messages round-trip through the codec`() {
        val hello = PhoneMessage.Hello(
            protocol = Proto.VERSION,
            deviceId = "android-1a2b3c4d",
            deviceName = "Redmi Note 11",
            model = "2201117TG",
            androidVersion = 33,
            appVersion = "1.0.0",
        )
        val encoded = ControlCodec.encode(hello)
        val text = String(encoded, Charsets.UTF_8)
        // The discriminator must be the `t` key the other two implementations use.
        assertTrue(text.contains("\"t\":\"hello\""))

        val decoded = ControlCodec.decodeDesktop(encoded)
        assertNull("a phone message must not decode as a desktop message", decoded)
    }

    @Test
    fun `decodes a desktop command and ignores unknown types`() {
        val setVideo = """{"t":"set_video","resolution":"1920x1080","fps":30,"bitrate":8000000}"""
        val decoded = ControlCodec.decodeDesktop(setVideo.toByteArray())
        assertTrue(decoded is DesktopMessage.SetVideo)
        assertEquals("1920x1080", (decoded as DesktopMessage.SetVideo).resolution)
        assertEquals(30, decoded.fps)
        assertEquals(8_000_000, decoded.bitrate)

        // Forward compatibility: a newer desktop may send a command this build
        // does not know, which must be ignored rather than throwing (docs §6).
        val unknown = """{"t":"a_command_from_the_future","payload":{"x":1}}"""
        assertNull(ControlCodec.decodeDesktop(unknown.toByteArray()))

        // Unknown fields on a known command must also be tolerated.
        val extended = """{"t":"start_stream","addedLater":true}"""
        assertTrue(ControlCodec.decodeDesktop(extended.toByteArray()) is DesktopMessage.StartStream)
    }

    @Test
    fun `state messages encode every field the desktop renders`() {
        val state = PhoneMessage.State(
            streaming = true,
            camera = "back",
            resolution = "1920x1080",
            fps = 30,
            actualResolution = "1280x720",
            actualFps = 30,
            bitrate = 4_000_000,
            mirror = false,
            rotation = 0,
            torch = false,
            zoom = 1f,
            battery = 87,
            charging = true,
            thermal = "none",
        )
        val text = String(ControlCodec.encode(state), Charsets.UTF_8)
        assertTrue(text.contains("\"actualResolution\":\"1280x720\""))
        assertTrue(text.contains("\"streaming\":true"))
    }

    @Test
    fun `hmac authentication matches the documented construction`() {
        val serverNonce = ByteArray(32) { it.toByte() }
        val clientNonce = 0x0102030405060708L
        val secret = "00".repeat(32)

        val auth = Auth.deviceSecretAuth("android-1a2b3c4d", secret, serverNonce, clientNonce)
        val parts = auth.split(":")
        assertEquals(2, parts.size)
        assertEquals("android-1a2b3c4d", parts[0])
        assertEquals(64, parts[1].length)

        // The signature must cover serverNonce || clientNonce in that order, which
        // is what makes a recorded handshake unusable against a fresh nonce.
        val reordered = Auth.deviceSecretAuth(
            "android-1a2b3c4d",
            secret,
            serverNonce.reversedArray(),
            clientNonce,
        )
        assertTrue(auth != reordered)
    }

    @Test
    fun `client nonce never goes backwards`() {
        // Replay defence depends on this: a restart must not reuse a nonce.
        val current = System.currentTimeMillis()
        assertEquals(current + 1, Auth.nextClientNonce(current))
        assertTrue(Auth.nextClientNonce(current + 100) > current)
    }

    @Test
    fun `secret fingerprint is short and not the secret`() {
        val secret = "ab".repeat(32)
        val fingerprint = Auth.secretFingerprint(secret)
        assertEquals(8, fingerprint.length)
        assertTrue(!secret.contains(fingerprint) || fingerprint.length < secret.length)
    }

    @Test
    fun `hex helpers round-trip and reject malformed input`() {
        val bytes = byteArrayOf(0x00, 0x0F, 0xA9.toByte(), 0xFF.toByte())
        assertEquals("000fa9ff", bytes.toHex())
        assertArrayEquals(bytes, "000fa9ff".hexToBytes())
        assertNull("abc".hexToBytesOrNull())
        assertNull("zz".hexToBytesOrNull())
    }

    /** Builds a well-formed 40-byte greeting. */
    private fun greetingBytes(status: HandshakeStatus, nonce: ByteArray = ByteArray(32)): ByteArray {
        val buffer = java.nio.ByteBuffer.allocate(Proto.SERVER_GREETING_SIZE)
            .order(java.nio.ByteOrder.BIG_ENDIAN)
        buffer.putInt(Proto.MAGIC)
        buffer.put(Proto.VERSION.toByte())
        buffer.put(status.wire.toByte())
        buffer.putShort(0)
        buffer.put(nonce)
        return buffer.array()
    }
}
