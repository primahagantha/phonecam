package com.phonecam.camera.session

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The rules that decide whether an *applied* setting may still be reported.
 *
 * The phone applies `set_screen` and `set_torch` only while it is streaming, and
 * samples its battery and temperature on the once-per-second stats tick, which
 * runs only while it is streaming. Outside a stream each of those is a fact about
 * a stream that has ended, and the Output card and the health banner used to
 * render them from the raw fields anyway: a stopped stream still read
 * "Phone screen: Black" and "Torch: On", and a phone that had cooled still
 * advised "Your phone is warming up". These rules are the phase guard every
 * sibling "actual" readout already carries ([connectedTransport],
 * `actualResolutionLabel`), kept pure so they are testable without a live
 * session, a camera or a window.
 */
class SessionStateAppliedTest {

    @Test
    fun `the screen mode is only reported while the stream that applies it runs`() {
        val streaming = SessionState(phase = SessionState.Phase.STREAMING, screenMode = "black")
        val connected = SessionState(phase = SessionState.Phase.CONNECTED, screenMode = "black")

        // The mode is applied by a window override that is scoped to streaming,
        // so once the stream stops the screen is back to normal whatever the
        // desktop last asked for.
        assertEquals("black", streaming.appliedScreenMode())
        assertEquals("keepOn", connected.appliedScreenMode())
    }

    @Test
    fun `a dim or black mode is never reported outside a stream`() {
        for (mode in listOf("dim", "black")) {
            assertEquals(
                "keepOn",
                SessionState(phase = SessionState.Phase.RECONNECTING, screenMode = mode)
                    .appliedScreenMode(),
            )
        }
    }

    @Test
    fun `the torch is only reported as on while a stream lights it`() {
        val streaming = SessionState(phase = SessionState.Phase.STREAMING, torch = true)
        val connected = SessionState(phase = SessionState.Phase.CONNECTED, torch = true)

        // The torch lives in the camera's repeating request, which dies with the
        // stream, so "On" must not outlive it.
        assertTrue(streaming.appliedTorch())
        assertFalse(connected.appliedTorch())
    }

    @Test
    fun `a torch that was never lit is not reported`() {
        assertFalse(SessionState(phase = SessionState.Phase.STREAMING, torch = false).appliedTorch())
    }

    @Test
    fun `health readings are only live while the stats tick runs`() {
        // The tick is started with the stream and stopped with it, so the last
        // sample is stale the moment the phase leaves STREAMING.
        assertTrue(SessionState(phase = SessionState.Phase.STREAMING).hasLiveHealth)
        assertFalse(SessionState(phase = SessionState.Phase.CONNECTED).hasLiveHealth)
        assertFalse(SessionState(phase = SessionState.Phase.IDLE).hasLiveHealth)
        assertFalse(SessionState(phase = SessionState.Phase.RECONNECTING).hasLiveHealth)
        assertFalse(SessionState(phase = SessionState.Phase.ERROR).hasLiveHealth)
    }

    @Test
    fun `the tap hint is only shown when a blackout surface hides the controls`() {
        // "Tap to show controls" describes exactly the black screen mode, where a
        // blackout surface covers the UI and a tap reveals it. In every other
        // mode the overlay is already visible, so the hint promised an
        // interaction that did not exist.
        assertTrue(
            SessionState(phase = SessionState.Phase.STREAMING, screenMode = "black")
                .overlayHintVisible(),
        )
        assertFalse(
            SessionState(phase = SessionState.Phase.STREAMING, screenMode = "keepOn")
                .overlayHintVisible(),
        )
        assertFalse(
            SessionState(phase = SessionState.Phase.STREAMING, screenMode = "dim")
                .overlayHintVisible(),
        )
        // Outside a stream the black mode is released (see appliedScreenMode), so
        // the hint must not be shown for a blackout that is not in force.
        assertFalse(
            SessionState(phase = SessionState.Phase.CONNECTED, screenMode = "black")
                .overlayHintVisible(),
        )
    }
}
