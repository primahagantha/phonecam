package com.phonecam.camera.ui.components

import com.phonecam.camera.transport.DiscoveredServer
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The rule that reads a discovery beacon's flags.
 *
 * The row's fallback used to render "Ready" exactly when the beacon set none of
 * busy / knowsThisDevice / pairingOpen, which claims a connectability the beacon
 * does not establish — connecting still requires pairing. The fallback is now the
 * neutral "Detected", and the token colour comes from the same classifier so the
 * words and the colour cannot disagree. These cases pin the precedence and the
 * fallback.
 */
class DiscoveryAvailabilityTest {

    private fun server(
        busy: Boolean = false,
        knowsThisDevice: Boolean = false,
        pairingOpen: Boolean = false,
    ) = DiscoveredServer(
        serverId = "server-1",
        name = "DESKTOP-ABC",
        host = "192.168.1.24",
        port = 8765,
        pairingOpen = pairingOpen,
        knowsThisDevice = knowsThisDevice,
        busy = busy,
        lastSeenMs = 0L,
    )

    @Test
    fun `a beacon with no flags only establishes presence`() {
        // The defect: this case used to render "Ready".
        assertEquals(DiscoveredAvailability.Present, discoveredAvailability(server()))
    }

    @Test
    fun `busy wins over the other flags`() {
        assertEquals(
            DiscoveredAvailability.Busy,
            discoveredAvailability(server(busy = true, knowsThisDevice = true, pairingOpen = true)),
        )
    }

    @Test
    fun `knowing this device wins over an open pairing window`() {
        assertEquals(
            DiscoveredAvailability.Known,
            discoveredAvailability(server(knowsThisDevice = true, pairingOpen = true)),
        )
    }

    @Test
    fun `an open pairing window is reported when nothing else is set`() {
        assertEquals(DiscoveredAvailability.PairingOpen, discoveredAvailability(server(pairingOpen = true)))
    }
}
