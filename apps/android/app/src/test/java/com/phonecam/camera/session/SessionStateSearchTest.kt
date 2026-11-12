package com.phonecam.camera.session

import com.phonecam.camera.transport.DiscoveredServer
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The search feedback the user sees.
 *
 * Discovery is silent: a probe nobody answers and a probe never sent leave the
 * same evidence behind, so the screen can only distinguish them if the state
 * machine says which one happened. These are the transitions that decide whether
 * the button is disabled, whether the spinner is up, and whether the empty-state
 * text asks the user to act.
 */
class SessionStateSearchTest {

    private fun server(id: String) = DiscoveredServer(
        serverId = id,
        name = "DESKTOP-$id",
        host = "192.168.1.24",
        port = 48721,
        pairingOpen = true,
        knowsThisDevice = false,
        busy = false,
        lastSeenMs = 0L,
    )

    @Test
    fun `a fresh session has not searched`() {
        assertEquals(SearchState.Idle, SessionState().search)
    }

    @Test
    fun `starting a search disables the button`() {
        assertEquals(SearchState.Searching, SessionState().searchStarted().search)
    }

    @Test
    fun `a search that nobody answered reports not found`() {
        val done = SessionState().searchStarted().searchCompleted()

        assertEquals(SearchState.NotFound, done.search)
    }

    @Test
    fun `a search that found PCs reports how many`() {
        val state = SessionState(
            discovered = listOf(server("a"), server("b")),
        ).searchStarted()

        assertEquals(SearchState.Found(2), state.searchCompleted().search)
    }

    @Test
    fun `the count follows the list the user can actually connect to`() {
        // A PC that answered and was then aged out of `discovered` must not be
        // announced: "Found 1 PC" over an empty list is worse than "not found".
        val state = SessionState(
            discovered = listOf(server("a")),
        ).searchStarted().searchCompleted()
        val aged = state.copy(discovered = emptyList())

        assertEquals(SearchState.NotFound, aged.searchCompleted().search)
    }

    @Test
    fun `an outcome fades back to idle so it cannot become a permanent verdict`() {
        val found = SessionState(discovered = listOf(server("a"))).searchCompleted()
        val missing = SessionState().searchCompleted()

        assertEquals(SearchState.Idle, found.searchResultCleared().search)
        assertEquals(SearchState.Idle, missing.searchResultCleared().search)
    }

    @Test
    fun `fading leaves a search that is still running alone`() {
        val running = SessionState().searchStarted()

        assertEquals(SearchState.Searching, running.searchResultCleared().search)
    }

    @Test
    fun `a search restores the phase it interrupted`() {
        val before = SessionState(phase = SessionState.Phase.CONNECTED)
        val searching = before.searchStarted().copy(phase = SessionState.Phase.DISCOVERING)

        assertEquals(SessionState.Phase.CONNECTED, searching.searchPhaseRestored(before.phase).phase)
    }

    @Test
    fun `a search does not stomp a connection that started while it ran`() {
        // The user tapped search and then tapped Connect on a PC that appeared;
        // the timer expiring must not drag the screen back to Connected/Idle.
        val connected = SessionState(phase = SessionState.Phase.CONNECTING)

        assertEquals(
            SessionState.Phase.CONNECTING,
            connected.searchPhaseRestored(SessionState.Phase.IDLE).phase,
        )
    }
}
