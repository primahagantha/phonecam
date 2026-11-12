package com.phonecam.camera.session

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The rule that a stream teardown cannot overwrite the connection's state.
 *
 * A teardown describes the stream, not the connection: losing the picture is not
 * losing the PC, so with the control channel open the session falls back to
 * CONNECTED. But RECONNECTING and ERROR are later, different facts about the
 * connection, and the phone was seen resetting them to IDLE when the control
 * reader reported a close that a deliberate teardown had already handled. The
 * duplicate report is prevented by cancelling the reader before its socket is
 * closed; this rule is the other half of the same guarantee, kept pure so it is
 * testable without a live session.
 */
class SessionStateTeardownTest {

    @Test
    fun `a teardown during a retry does not reset the retry state to idle`() {
        val reconnecting = SessionState(phase = SessionState.Phase.RECONNECTING)

        val stopped = reconnecting.afterStreamStopped(controlOpen = false)

        assertEquals(SessionState.Phase.RECONNECTING, stopped.phase)
    }

    @Test
    fun `a teardown after the terminal failure does not clear the error`() {
        val failed = SessionState(phase = SessionState.Phase.CONNECTING)
            .failed("Lost the connection to your PC and couldn't get it back.")

        val stopped = failed.afterStreamStopped(controlOpen = false)

        assertEquals(SessionState.Phase.ERROR, stopped.phase)
        assertEquals("Lost the connection to your PC and couldn't get it back.", stopped.error)
    }

    @Test
    fun `a stream teardown with the control channel open falls back to connected`() {
        val streaming = SessionState(phase = SessionState.Phase.STREAMING)

        val stopped = streaming.afterStreamStopped(controlOpen = true)

        assertEquals(SessionState.Phase.CONNECTED, stopped.phase)
    }

    @Test
    fun `a stream teardown without a control channel falls back to idle`() {
        val streaming = SessionState(phase = SessionState.Phase.STREAMING)

        val stopped = streaming.afterStreamStopped(controlOpen = false)

        assertEquals(SessionState.Phase.IDLE, stopped.phase)
    }

    @Test
    fun `a teardown keeps the notice and error it did not create`() {
        val reconnecting = SessionState(phase = SessionState.Phase.RECONNECTING)

        val stopped = reconnecting.afterStreamStopped(controlOpen = false)

        assertEquals(reconnecting, stopped)
    }
}
