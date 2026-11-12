package com.phonecam.camera.protocol

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.security.MessageDigest
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

class AuthTest {

    @Test
    fun `randomHex generates expected lowercase hex length and uniqueness`() {
        val hex16 = Auth.randomHex(16)
        val hex32 = Auth.randomHex(32)
        val hex0 = Auth.randomHex(0)

        assertEquals(32, hex16.length)
        assertEquals(64, hex32.length)
        assertEquals("", hex0)
        assertTrue(hex16.matches(Regex("^[0-9a-f]{32}$")))
        assertTrue(hex32.matches(Regex("^[0-9a-f]{64}$")))
        assertNotEquals(Auth.randomHex(16), hex16)
    }

    @Test
    fun `randomHex rejects negative byte length`() {
        try {
            Auth.randomHex(-1)
            fail("expected IllegalArgumentException for negative length")
        } catch (_: IllegalArgumentException) {
            // expected
        }
    }

    @Test
    fun `deviceSecretAuth matches JVM HmacSHA256 vector`() {
        val deviceId = "android-deadbeef"
        val secretHex = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
        val serverNonce = ByteArray(32) { (0xA0 + it).toByte() }
        val clientNonce = 0x0102030405060708L

        val actual = Auth.deviceSecretAuth(deviceId, secretHex, serverNonce, clientNonce)

        val mac = Mac.getInstance("HmacSHA256").apply {
            init(SecretKeySpec(secretHex.hexToBytes(), "HmacSHA256"))
        }
        val expectedHex = mac.doFinal(serverNonce + clientNonce.toBigEndianBytes()).toHex()
        assertEquals("$deviceId:$expectedHex", actual)
    }

    @Test
    fun `pairRequestAuth uses device secret when provided and default pair-request key when null`() {
        val deviceId = "android-12345678"
        val serverNonce = ByteArray(32) { (it * 5).toByte() }
        val clientNonce = 42L

        val fallbackActual = Auth.pairRequestAuth(deviceId, null, serverNonce, clientNonce)
        val fallbackKey = "phonecam-pair-request".toByteArray(Charsets.US_ASCII)
        val fallbackMac = Mac.getInstance("HmacSHA256").apply {
            init(SecretKeySpec(fallbackKey, "HmacSHA256"))
        }
        val fallbackExpected = fallbackMac.doFinal(serverNonce + clientNonce.toBigEndianBytes()).toHex()
        assertEquals("$deviceId:request:$fallbackExpected", fallbackActual)

        val secretHex = "ab".repeat(32)
        val secretActual = Auth.pairRequestAuth(deviceId, secretHex, serverNonce, clientNonce)
        val secretMac = Mac.getInstance("HmacSHA256").apply {
            init(SecretKeySpec(secretHex.hexToBytes(), "HmacSHA256"))
        }
        val secretExpected = secretMac.doFinal(serverNonce + clientNonce.toBigEndianBytes()).toHex()
        assertEquals("$deviceId:request:$secretExpected", secretActual)
        assertNotEquals(fallbackActual, secretActual)
    }

    @Test
    fun `auth methods reject malformed hex secrets`() {
        val nonce = ByteArray(32)
        for (badHex in listOf("abc", "zzzz", "12 34")) {
            try {
                Auth.deviceSecretAuth("dev", badHex, nonce, 1L)
                fail("expected IllegalArgumentException for malformed hex '$badHex'")
            } catch (_: IllegalArgumentException) {
                // expected
            }

            try {
                Auth.pairRequestAuth("dev", badHex, nonce, 1L)
                fail("expected IllegalArgumentException for malformed hex '$badHex'")
            } catch (_: IllegalArgumentException) {
                // expected
            }
        }
    }

    @Test
    fun `secretFingerprint returns first 8 hex chars of sha256 of ascii secret`() {
        val secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        val expected = MessageDigest.getInstance("SHA-256")
            .digest(secret.toByteArray(Charsets.US_ASCII))
            .toHex()
            .take(8)
        assertEquals(expected, Auth.secretFingerprint(secret))
    }
}
