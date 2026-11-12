package com.phonecam.camera.session

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The rule that decides which transport the phone is using.
 *
 * USB mode is `adb reverse` (docs/protocol.md §1): the desktop core runs the
 * tunnel and sends the phone an intent to dial `127.0.0.1` on the service port,
 * so a loopback peer is the USB cable. The field this feeds used to default to
 * `"lan"` and nothing ever assigned it, so every USB session read "Local network"
 * on the phone and the USB toast was unreachable. The rule is kept pure so this
 * cannot regress unnoticed — a live USB session is not available to a unit test,
 * but the decision that was wrong is.
 */
class TransportForHostTest {

    @Test
    fun `the loopback address is the USB tunnel`() {
        assertEquals("usb", transportForHost("127.0.0.1"))
    }

    @Test
    fun `the localhost name is the USB tunnel`() {
        assertEquals("usb", transportForHost("localhost"))
    }

    @Test
    fun `the IPv6 loopback is the USB tunnel`() {
        assertEquals("usb", transportForHost("::1"))
    }

    @Test
    fun `a LAN address is the local network`() {
        assertEquals("lan", transportForHost("192.168.100.5"))
    }

    @Test
    fun `another loopback-range address is not treated as USB`() {
        // Only the exact loopback addresses the core dials are USB; 127.0.0.2 is
        // a loopback address too, but the core never sends the phone there, and
        // guessing that any 127/8 address means USB would mislabel a LAN dial to
        // a host that happens to use that range.
        assertEquals("lan", transportForHost("127.0.0.2"))
    }

    @Test
    fun `case and surrounding space do not change the decision`() {
        assertEquals("usb", transportForHost(" LocalHost "))
        assertEquals("usb", transportForHost("127.0.0.1 "))
    }
}
