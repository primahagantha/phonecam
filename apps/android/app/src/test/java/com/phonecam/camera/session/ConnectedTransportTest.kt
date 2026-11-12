package com.phonecam.camera.session

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The rule that decides when a link is real enough to announce.
 *
 * The "Connected over the cable/Wi-Fi" toast used to key off the transport string
 * alone, and `dial` sets that string the moment a connection starts. A USB dial
 * to a PC that was not there therefore announced a link that never existed, right
 * before the error. These two rules separate "a dial is in flight" from "a link
 * is up" and stop the same link being announced twice across a reconnect. Kept
 * pure because a live session — let alone a failed dial — is not available to a
 * unit test, but the decision that was wrong is.
 */
class ConnectedTransportTest {

    private fun state(phase: SessionState.Phase, transport: String) =
        SessionState(phase = phase, transport = transport)

    @Test
    fun `a connecting dial is not a link`() {
        // The bug: dial sets the transport to usb and the phase to CONNECTING at
        // once, so a toast keyed on the transport alone fired here.
        assertNull(state(SessionState.Phase.CONNECTING, "usb").connectedTransport())
        assertNull(state(SessionState.Phase.CONNECTING, "lan").connectedTransport())
    }

    @Test
    fun `an idle, searching, reconnecting or failed session has no link`() {
        assertNull(state(SessionState.Phase.IDLE, "usb").connectedTransport())
        assertNull(state(SessionState.Phase.DISCOVERING, "lan").connectedTransport())
        assertNull(state(SessionState.Phase.RECONNECTING, "usb").connectedTransport())
        assertNull(state(SessionState.Phase.ERROR, "usb").connectedTransport())
    }

    @Test
    fun `a connected USB session is a USB link`() {
        assertEquals("usb", state(SessionState.Phase.CONNECTED, "usb").connectedTransport())
        assertEquals("usb", state(SessionState.Phase.STREAMING, "usb").connectedTransport())
    }

    @Test
    fun `a connected LAN session is a LAN link`() {
        assertEquals("lan", state(SessionState.Phase.CONNECTED, "lan").connectedTransport())
        assertEquals("lan", state(SessionState.Phase.STREAMING, "lan").connectedTransport())
    }

    @Test
    fun `a connected session with a neutral transport is not a link`() {
        assertNull(state(SessionState.Phase.CONNECTED, "none").connectedTransport())
    }

    @Test
    fun `a teardown forgets the announced transport`() {
        assertNull(state(SessionState.Phase.IDLE, "none").rememberedTransport("usb"))
    }

    @Test
    fun `a live link remembers its transport`() {
        assertEquals("usb", state(SessionState.Phase.CONNECTED, "usb").rememberedTransport(null))
    }

    @Test
    fun `a dial in flight keeps the last remembered transport`() {
        // A reconnect dials before the link is up; the remembered value must
        // survive so the returning link is not announced a second time.
        assertEquals("usb", state(SessionState.Phase.CONNECTING, "usb").rememberedTransport("usb"))
        assertEquals("lan", state(SessionState.Phase.RECONNECTING, "lan").rememberedTransport("lan"))
    }

    @Test
    fun `an error keeps the last remembered transport`() {
        assertEquals("usb", state(SessionState.Phase.ERROR, "usb").rememberedTransport("usb"))
    }
}
