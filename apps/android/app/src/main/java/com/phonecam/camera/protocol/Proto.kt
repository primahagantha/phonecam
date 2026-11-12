package com.phonecam.camera.protocol

import com.phonecam.camera.nativecore.NativeCore
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.EOFException
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Kotlin mirror of `docs/protocol.md` v1.
 *
 * Kept byte-identical with `packages/shared-types/src/protocol.ts` and
 * `native/core/include/phonecam/protocol.hpp`. All multi-byte integers are
 * big-endian on the wire.
 */
object Proto {
    const val VERSION = 1

    const val DISCOVERY_PORT = 48720
    const val SERVICE_PORT = 48721

    const val MAX_CONTROL_JSON = 1024 * 1024
    const val MAX_VIDEO_PAYLOAD = 8 * 1024 * 1024

    const val PAIRING_TOKEN_TTL_MS = 300_000L
    const val BEACON_INTERVAL_MS = 1000L

    const val DEFAULT_BITRATE_720P = 4_000_000
    const val DEFAULT_BITRATE_1080P = 8_000_000

    /** `"PCAM"` as a big-endian u32. */
    const val MAGIC = 0x5043414D

    const val SERVER_GREETING_SIZE = 40
    const val CLIENT_HANDSHAKE_SIZE = 20
    const val SERVER_RESPONSE_SIZE = 24
    const val VIDEO_HEADER_SIZE = 24

    const val MAX_HANDSHAKE_AUTH = 512
}

enum class ChannelRole(val wire: Int) {
    CONTROL(1),
    VIDEO(2),
}

enum class AuthMode(val wire: Int) {
    /** Accepted by the desktop only from 127.0.0.1. */
    LOOPBACK(0),

    /** One-time 32-hex-char token scanned from the desktop QR code. */
    PAIRING_TOKEN(1),

    /** `deviceId:hexHmac` proving possession of the stored device secret. */
    DEVICE_SECRET(2),

    /** `deviceId:request:hexHmac` — raises an approval prompt on the desktop. */
    PAIR_REQUEST(3),
}

enum class HandshakeStatus(val wire: Int) {
    OK(0),
    DENIED(1),
    VERSION_MISMATCH(2),
    BAD_REQUEST(3),
    BUSY(4),
    TOKEN_EXPIRED(5),
    APPROVAL_PENDING(6),
    UNAUTHORIZED_SOURCE(7),
    ;

    companion object {
        fun fromWire(value: Int): HandshakeStatus =
            entries.firstOrNull { it.wire == value } ?: BAD_REQUEST
    }
}

object VideoFlags {
    const val NONE = 0
    const val KEYFRAME = 1 shl 0
    const val CODEC_CONFIG = 1 shl 1
    const val END_OF_STREAM = 1 shl 2
}

data class ServerGreeting(
    val protoVersion: Int,
    val status: HandshakeStatus,
    val serverNonce: ByteArray,
) {
    override fun equals(other: Any?): Boolean =
        other is ServerGreeting &&
            protoVersion == other.protoVersion &&
            status == other.status &&
            serverNonce.contentEquals(other.serverNonce)

    override fun hashCode(): Int =
        (protoVersion * 31 + status.wire) * 31 + serverNonce.contentHashCode()
}

data class ServerResponse(
    val protoVersion: Int,
    val status: HandshakeStatus,
    val role: Int,
    val body: String,
) {
    /** Parses [body] as JSON; returns `null` for empty or malformed bodies. */
    val json: kotlinx.serialization.json.JsonObject?
        get() = if (body.isEmpty()) null else runCatching {
            kotlinx.serialization.json.Json.parseToJsonElement(body)
                as? kotlinx.serialization.json.JsonObject
        }.getOrNull()
}

data class VideoPacketHeader(
    val frameId: Long,
    val ptsUs: Long,
    val flags: Int,
    val codec: Int,
    val payloadLen: Int,
)

/**
 * Wire framing for the phone-facing channels.
 *
 * The phone is always the client: it reads the desktop's greeting, writes its
 * handshake, then reads the response. Video and control use separate TCP
 * connections, each opened with its own handshake.
 */
object Framing {

    @Throws(IOException::class)
    fun readGreeting(input: DataInputStream): ServerGreeting {
        val buf = ByteArray(Proto.SERVER_GREETING_SIZE)
        input.readFully(buf)
        val decoded = NativeCore.decodeServerGreeting(buf)
        if (decoded.size != 34) {
            throw IOException("invalid decoded greeting length: ${decoded.size}")
        }
        val version = decoded[0].toInt() and 0xFF
        val status = HandshakeStatus.fromWire(decoded[1].toInt() and 0xFF)
        val nonce = decoded.copyOfRange(2, 34)
        return ServerGreeting(version, status, nonce)
    }

    @Throws(IOException::class)
    fun writeHandshake(
        output: DataOutputStream,
        role: ChannelRole,
        authMode: AuthMode,
        clientNonce: Long,
        auth: String,
    ) {
        val authBytes = auth.toByteArray(Charsets.US_ASCII)
        require(authBytes.size <= Proto.MAX_HANDSHAKE_AUTH) { "auth payload too long" }
        val packet = NativeCore.encodeClientHandshake(
            role = role.wire,
            authMode = authMode.wire,
            clientNonce = clientNonce,
            authBytes = authBytes,
        )
        output.write(packet)
        output.flush()
    }

    @Throws(IOException::class)
    fun readResponse(input: DataInputStream): ServerResponse {
        val fixed = ByteArray(Proto.SERVER_RESPONSE_SIZE)
        input.readFully(fixed)
        val header = NativeCore.decodeServerResponseHeader(fixed)
        if (header.size < 4) {
            throw IOException("invalid decoded server response header")
        }
        val version = header[0]
        val status = HandshakeStatus.fromWire(header[1])
        val role = header[2]
        val jsonLen = header[3]
        if (jsonLen < 0 || jsonLen > Proto.MAX_CONTROL_JSON) {
            throw IOException("server response json length out of range: $jsonLen")
        }
        val body = ByteArray(jsonLen)
        input.readFully(body)
        return ServerResponse(version, status, role, String(body, Charsets.UTF_8))
    }

    @Throws(IOException::class)
    fun writeControl(output: DataOutputStream, json: ByteArray) {
        if (json.size > Proto.MAX_CONTROL_JSON) {
            throw IOException("control message too large: ${json.size}")
        }
        output.writeInt(json.size)
        output.write(json)
        output.flush()
    }

    @Throws(IOException::class)
    fun readControl(input: DataInputStream): ByteArray {
        val length = try {
            input.readInt()
        } catch (eof: EOFException) {
            throw IOException("control channel closed", eof)
        }
        if (length < 0 || length > Proto.MAX_CONTROL_JSON) {
            throw IOException("control frame length out of range: $length")
        }
        val payload = ByteArray(length)
        input.readFully(payload)
        return payload
    }

    /**
     * Writes `u32 totalLen` + 24-byte header + payload.
     *
     * [payload] is a H.264 Annex-B access unit. Callers must not copy the
     * payload into a larger buffer to avoid doubling per-frame allocations on
     * the encoder output path.
     */
    @Throws(IOException::class)
    fun writeVideoPacket(
        output: DataOutputStream,
        header: VideoPacketHeader,
        payload: ByteArray,
        payloadOffset: Int = 0,
        payloadLength: Int = payload.size,
    ) {
        if (payloadOffset < 0 || payloadLength < 0 || payloadOffset + payloadLength > payload.size) {
            throw IOException("video payload slice out of bounds: offset=$payloadOffset length=$payloadLength size=${payload.size}")
        }
        val prefix = NativeCore.encodeVideoPacketHeader(
            frameId = header.frameId,
            ptsUs = header.ptsUs,
            flags = header.flags,
            codec = header.codec,
            payloadLen = payloadLength,
        )
        output.write(prefix)
        output.write(payload, payloadOffset, payloadLength)
        output.flush()
    }

    @Throws(IOException::class)
    fun readVideoHeader(buffer: ByteBuffer): VideoPacketHeader {
        buffer.order(ByteOrder.BIG_ENDIAN)
        if (buffer.remaining() < Proto.VIDEO_HEADER_SIZE) {
            throw IOException("video header truncated: ${buffer.remaining()} bytes")
        }
        val raw = ByteArray(Proto.VIDEO_HEADER_SIZE)
        buffer.get(raw)
        val fields = NativeCore.decodeVideoPacketHeader(raw)
        if (fields.size < 5) {
            throw IOException("invalid decoded video packet header")
        }
        return VideoPacketHeader(
            frameId = fields[0],
            ptsUs = fields[1],
            flags = fields[2].toInt(),
            codec = fields[3].toInt(),
            payloadLen = fields[4].toInt(),
        )
    }
}
