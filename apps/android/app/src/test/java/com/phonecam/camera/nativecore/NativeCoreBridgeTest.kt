package com.phonecam.camera.nativecore

import com.phonecam.camera.protocol.AuthMode
import com.phonecam.camera.protocol.ChannelRole
import com.phonecam.camera.protocol.HandshakeStatus
import com.phonecam.camera.protocol.Proto
import com.phonecam.camera.protocol.VideoFlags
import com.phonecam.camera.transport.DiscoveryClient
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.ByteOrder

class NativeCoreBridgeTest {

    @Test
    fun `nativeVersion returns compiled rust core identifier`() {
        val version = NativeCore.nativeVersion()
        assertTrue("expected phonecam-android-core in '$version'", version.contains("phonecam-android-core"))
        assertTrue("expected protocol v1 in '$version'", version.contains("protocol v1"))
    }

    @Test
    fun `decodeServerGreeting decodes version status and nonce and rejects bad magic`() {
        val nonce = ByteArray(32) { (it * 3 + 1).toByte() }
        val greeting = ByteBuffer.allocate(Proto.SERVER_GREETING_SIZE)
            .order(ByteOrder.BIG_ENDIAN)
            .putInt(Proto.MAGIC)
            .put(Proto.VERSION.toByte())
            .put(HandshakeStatus.BUSY.wire.toByte())
            .putShort(0)
            .put(nonce)
            .array()

        val decoded = NativeCore.decodeServerGreeting(greeting)
        assertEquals(34, decoded.size)
        assertEquals(Proto.VERSION, decoded[0].toInt() and 0xFF)
        assertEquals(HandshakeStatus.BUSY.wire, decoded[1].toInt() and 0xFF)
        assertArrayEquals(nonce, decoded.copyOfRange(2, 34))

        val badMagic = greeting.copyOf().apply { this[0] = 'Z'.code.toByte() }
        try {
            NativeCore.decodeServerGreeting(badMagic)
            fail("expected IOException on bad greeting magic")
        } catch (expected: IOException) {
            assertTrue(expected.message.orEmpty().contains("PhoneCam"))
        }

        try {
            NativeCore.decodeServerGreeting(ByteArray(12))
            fail("expected IOException on truncated greeting")
        } catch (expected: IOException) {
            assertTrue(expected.message.orEmpty().contains("PhoneCam"))
        }
    }

    @Test
    fun `encodeClientHandshake builds big endian header and rejects oversized auth`() {
        val authBytes = "android-1234:deadbeef".toByteArray(Charsets.US_ASCII)
        val encoded = NativeCore.encodeClientHandshake(
            role = ChannelRole.CONTROL.wire,
            authMode = AuthMode.DEVICE_SECRET.wire,
            clientNonce = 0x1122334455667788L,
            authBytes = authBytes,
        )
        assertEquals(Proto.CLIENT_HANDSHAKE_SIZE + authBytes.size, encoded.size)

        val buf = ByteBuffer.wrap(encoded).order(ByteOrder.BIG_ENDIAN)
        assertEquals(Proto.MAGIC, buf.int)
        assertEquals(Proto.VERSION, buf.get().toInt() and 0xFF)
        assertEquals(ChannelRole.CONTROL.wire, buf.get().toInt() and 0xFF)
        assertEquals(AuthMode.DEVICE_SECRET.wire, buf.get().toInt() and 0xFF)
        assertEquals(0, buf.get().toInt() and 0xFF)
        assertEquals(0x1122334455667788L, buf.long)
        assertEquals(authBytes.size, buf.short.toInt() and 0xFFFF)
        assertEquals(0, buf.short.toInt() and 0xFFFF)

        try {
            NativeCore.encodeClientHandshake(
                role = ChannelRole.CONTROL.wire,
                authMode = AuthMode.PAIRING_TOKEN.wire,
                clientNonce = 1L,
                authBytes = ByteArray(Proto.MAX_HANDSHAKE_AUTH + 1),
            )
            fail("expected IllegalArgumentException for oversized handshake auth")
        } catch (_: IllegalArgumentException) {
            // expected
        }
    }

    @Test
    fun `decodeServerResponseHeader decodes fields and rejects bad length or magic`() {
        val header = ByteBuffer.allocate(Proto.SERVER_RESPONSE_SIZE)
            .order(ByteOrder.BIG_ENDIAN)
            .putInt(Proto.MAGIC)
            .put(Proto.VERSION.toByte())
            .put(HandshakeStatus.APPROVAL_PENDING.wire.toByte())
            .put(ChannelRole.CONTROL.wire.toByte())
            .put(0)
            .putLong(0L)
            .putInt(128)
            .putInt(0)
            .array()

        val decoded = NativeCore.decodeServerResponseHeader(header)
        assertArrayEquals(
            intArrayOf(
                Proto.VERSION,
                HandshakeStatus.APPROVAL_PENDING.wire,
                ChannelRole.CONTROL.wire,
                128,
            ),
            decoded,
        )

        val oversized = ByteBuffer.allocate(Proto.SERVER_RESPONSE_SIZE)
            .order(ByteOrder.BIG_ENDIAN)
            .putInt(Proto.MAGIC)
            .put(Proto.VERSION.toByte())
            .put(HandshakeStatus.OK.wire.toByte())
            .put(ChannelRole.CONTROL.wire.toByte())
            .put(0)
            .putLong(0L)
            .putInt(Proto.MAX_CONTROL_JSON + 1)
            .putInt(0)
            .array()
        try {
            NativeCore.decodeServerResponseHeader(oversized)
            fail("expected IOException on oversized response json length")
        } catch (_: IOException) {
            // expected
        }
    }

    @Test
    fun `encodeVideoPacket and decodeVideoPacketHeader round-trip slice and reject bad inputs`() {
        val backing = byteArrayOf(9, 9, 0x00, 0x00, 0x00, 0x01, 0x65, 0x88.toByte(), 7)
        val packet = NativeCore.encodeVideoPacket(
            frameId = 0xFFFF_FFFFL,
            ptsUs = 987_654_321L,
            flags = VideoFlags.KEYFRAME,
            codec = 1,
            payload = backing,
            offset = 2,
            length = 6,
        )
        assertEquals(4 + Proto.VIDEO_HEADER_SIZE + 6, packet.size)

        val buf = ByteBuffer.wrap(packet).order(ByteOrder.BIG_ENDIAN)
        assertEquals(Proto.VIDEO_HEADER_SIZE + 6, buf.int)

        val headerBytes = ByteArray(Proto.VIDEO_HEADER_SIZE).also { buf.get(it) }
        val decoded = NativeCore.decodeVideoPacketHeader(headerBytes)
        assertEquals(0xFFFF_FFFFL, decoded[0])
        assertEquals(987_654_321L, decoded[1])
        assertEquals(VideoFlags.KEYFRAME.toLong(), decoded[2])
        assertEquals(1L, decoded[3])
        assertEquals(6L, decoded[4])
        assertArrayEquals(
            byteArrayOf(0x00, 0x00, 0x00, 0x01, 0x65, 0x88.toByte()),
            packet.copyOfRange(4 + Proto.VIDEO_HEADER_SIZE, packet.size),
        )

        try {
            NativeCore.encodeVideoPacketHeader(1L, 0L, 0, 1, Proto.MAX_VIDEO_PAYLOAD + 1)
            fail("expected IOException on oversized video payload")
        } catch (_: IOException) {
            // expected
        }

        try {
            NativeCore.encodeVideoPacket(1L, 0L, 0, 1, backing, offset = 5, length = 10)
            fail("expected IndexOutOfBoundsException on out-of-range slice")
        } catch (_: IndexOutOfBoundsException) {
            // expected
        }
    }

    @Test
    fun `validateQrPayload normalizes valid payloads and rejects malformed input`() {
        val token = "0123456789abcdef0123456789abcdef"
        val validMinimal = """{"protocol":"phonecam","version":1,"host":"192.168.1.50","port":48721,"token":"$token","name":"   "}"""
        val normalized = NativeCore.validateQrPayload(validMinimal)
        assertNotNull(normalized)

        val json = Json.parseToJsonElement(normalized!!).jsonObject
        assertEquals("phonecam", json["protocol"]!!.jsonPrimitive.content)
        assertEquals(1, json["version"]!!.jsonPrimitive.intOrNull)
        assertEquals("192.168.1.50", json["host"]!!.jsonPrimitive.content)
        assertEquals(48721, json["port"]!!.jsonPrimitive.intOrNull)
        assertEquals(token, json["token"]!!.jsonPrimitive.content)
        assertEquals("null", json["name"].toString())
        assertEquals("null", json["serverId"].toString())

        assertNull(NativeCore.validateQrPayload("""{"protocol":"other","version":1,"host":"1.2.3.4","port":48721,"token":"$token"}"""))
        assertNull(NativeCore.validateQrPayload("""{"protocol":"phonecam","version":2,"host":"1.2.3.4","port":48721,"token":"$token"}"""))
        assertNull(NativeCore.validateQrPayload("""{"protocol":"phonecam","version":1,"host":"1.2.3.4","port":0,"token":"$token"}"""))
        assertNull(NativeCore.validateQrPayload("""{"protocol":"phonecam","version":1,"host":"1.2.3.4","port":48721,"token":"not-hex"}"""))
    }

    @Test
    fun `parseDiscoveryBeacon validates desktop beacons and rejects probes or invalid packets`() {
        val validBeacon = """
            {
              "t":"phonecam.beacon",
              "v":1,
              "role":"desktop",
              "serverId":"srv-01",
              "name":"Studio PC",
              "host":"192.168.1.10",
              "port":48721,
              "pairingOpen":true,
              "pairedDeviceIds":["android-aaa","android-bbb"],
              "busy":false,
              "version":"1.0.0"
            }
        """.trimIndent().toByteArray(Charsets.UTF_8)

        assertNotNull(NativeCore.parseDiscoveryBeacon(validBeacon))

        val server = DiscoveryClient.parseDesktopBeacon(
            jsonBytes = validBeacon,
            sourceHost = "192.168.1.10",
            deviceId = "android-bbb",
            nowMs = 42_000L,
        )
        assertNotNull(server)
        assertEquals("srv-01", server!!.serverId)
        assertEquals("Studio PC", server.name)
        assertEquals("192.168.1.10", server.host)
        assertEquals(48721, server.port)
        assertTrue(server.pairingOpen)
        assertTrue(server.knowsThisDevice)
        assertFalse(server.busy)
        assertEquals(42_000L, server.lastSeenMs)

        val probe = """{"t":"phonecam.probe","v":1,"role":"phone"}""".toByteArray(Charsets.UTF_8)
        assertNull(NativeCore.parseDiscoveryBeacon(probe))
        assertNull(DiscoveryClient.parseDesktopBeacon(probe, "192.168.1.11", "android-bbb"))

        val phoneBeacon = """{"t":"phonecam.beacon","v":1,"role":"phone","deviceId":"android-1"}"""
            .toByteArray(Charsets.UTF_8)
        assertNull(NativeCore.parseDiscoveryBeacon(phoneBeacon))

        val emptyServerId = """{"t":"phonecam.beacon","v":1,"role":"desktop","serverId":"","port":48721}"""
            .toByteArray(Charsets.UTF_8)
        assertNull(NativeCore.parseDiscoveryBeacon(emptyServerId))
    }
}
