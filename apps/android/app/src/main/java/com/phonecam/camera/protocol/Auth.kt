package com.phonecam.camera.protocol

import com.phonecam.camera.nativecore.NativeCore

/**
 * Authentication for the phone-facing channels (docs/protocol.md §4–§5).
 *
 * The phone never sends a bare IP + password. First contact uses either a
 * one-time QR token or an explicit desktop approval; afterwards the phone proves
 * possession of a stored 32-byte device secret with an HMAC over both nonces.
 *
 * Cryptographic operations delegate to the shared Rust core via [NativeCore].
 */
object Auth {

    fun randomHex(bytes: Int): String {
        require(bytes >= 0) { "bytes must be non-negative" }
        return NativeCore.randomHex(bytes)
    }

    /** A monotonic client nonce: never decreases, even across process restarts. */
    fun nextClientNonce(previous: Long): Long {
        val now = System.currentTimeMillis()
        return if (now > previous) now else previous + 1
    }

    /**
     * `deviceId:hexHmac` where the HMAC is over `serverNonce || clientNonce`
     * with the device secret as key (docs §4.4, auth mode 2).
     */
    fun deviceSecretAuth(
        deviceId: String,
        deviceSecretHex: String,
        serverNonce: ByteArray,
        clientNonce: Long,
    ): String = NativeCore.deviceSecretAuth(deviceId, deviceSecretHex, serverNonce, clientNonce)

    /**
     * `deviceId:request:hexHmac` (docs §4.3, auth mode 3). When the phone already
     * holds a device secret it signs with it, otherwise it signs with the fixed
     * pair-request key. Either is accepted by the desktop; the human approval is
     * what actually authorises the pairing.
     */
    fun pairRequestAuth(
        deviceId: String,
        deviceSecretHex: String?,
        serverNonce: ByteArray,
        clientNonce: Long,
    ): String = NativeCore.pairRequestAuth(deviceId, deviceSecretHex, serverNonce, clientNonce)

    /** Short correlator safe to write to logs; never the secret itself. */
    fun secretFingerprint(deviceSecretHex: String): String =
        NativeCore.secretFingerprint(deviceSecretHex)
}

fun ByteArray.toHex(): String {
    val digits = "0123456789abcdef"
    val out = StringBuilder(size * 2)
    for (byte in this) {
        val value = byte.toInt() and 0xFF
        out.append(digits[value ushr 4])
        out.append(digits[value and 0x0F])
    }
    return out.toString()
}

/** Returns `null` for odd-length or non-hex input rather than throwing. */
fun String.hexToBytesOrNull(): ByteArray? {
    if (length % 2 != 0) return null
    val out = ByteArray(length / 2)
    for (i in out.indices) {
        val high = Character.digit(this[i * 2], 16)
        val low = Character.digit(this[i * 2 + 1], 16)
        if (high < 0 || low < 0) return null
        out[i] = ((high shl 4) or low).toByte()
    }
    return out
}

fun String.hexToBytes(): ByteArray =
    hexToBytesOrNull() ?: throw IllegalArgumentException("not a hex string")

fun Long.toBigEndianBytes(): ByteArray {
    val out = ByteArray(8)
    for (i in 0 until 8) {
        out[i] = (this ushr ((7 - i) * 8)).toByte()
    }
    return out
}
