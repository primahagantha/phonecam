package com.phonecam.camera.ui

import com.phonecam.camera.protocol.Proto
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The QR parser is the gate between "a code" and "a PhoneCam pairing code", and
 * the pairing toast is only honest if it reads that gate the same way.
 *
 * The regression behind A4: a code with no `name` field parsed and was dispatched,
 * but the toast then fell back to a remembered PC's name (or, with neither,
 * claimed the code was invalid). `name` is optional in the payload, so these
 * cases pin that a name-less code is valid — and that a genuinely malformed one
 * is not, because that is the only case the "isn't a PhoneCam pairing code"
 * message is true for.
 */
class QrPayloadTest {

    private fun code(nameField: String = "", serverIdField: String = "") =
        """{"protocol":"phonecam","version":${Proto.VERSION},"host":"192.168.1.24","port":8765,""" +
            """"token":"${"a".repeat(32)}"$nameField$serverIdField}"""

    @Test
    fun `a code without a name is still a valid pairing code`() {
        // `name` is optional (docs/protocol.md §4.1). Parsing must succeed so the
        // dial is dispatched and the user is not told their valid code is invalid.
        val parsed = parseQrPayload(code())

        assertEquals("192.168.1.24", parsed?.host)
        assertEquals(8765, parsed?.port)
        assertNull(parsed?.name)
    }

    @Test
    fun `a code with a name carries it through`() {
        val parsed = parseQrPayload(code(nameField = ""","name":"DESKTOP-ABC""""))

        assertEquals("DESKTOP-ABC", parsed?.name)
    }

    @Test
    fun `a non-PhoneCam code is rejected`() {
        // The one case the error message is actually true for.
        assertNull(parseQrPayload("https://example.com/not-a-pairing-code"))
        assertNull(parseQrPayload("""{"protocol":"something-else","version":${Proto.VERSION}}"""))
    }

    @Test
    fun `a code with a malformed token is rejected`() {
        val badToken = """{"protocol":"phonecam","version":${Proto.VERSION},"host":"192.168.1.24","port":8765,"token":"nope"}"""
        assertNull(parseQrPayload(badToken))
    }
}
