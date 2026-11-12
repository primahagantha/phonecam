package com.phonecam.camera.session

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The phone's own viewfinder as a setting.
 *
 * The switch used to be a `MutableStateFlow` in the view model, seeded once from
 * the preference store and never re-read. That made it a second source of truth
 * for a fact the capture session acts on: the switch could show one position
 * while the camera had already been built with the other, and a process restart
 * (or a rotation that rebuilt the view model from the store while the service
 * still held the old value) put the two back out of step.
 *
 * The rule these tests pin is that the value lives in [SessionState] — the
 * service's state, which is also what the desktop and the notification read — and
 * that its default is the safe one.
 */
class PhonePreviewSettingTest {

    @Test
    fun `the viewfinder is on by default`() {
        // A webcam with no picture is hard to trust, and a user who has never
        // touched the setting must not be handed a phone that looks broken.
        assertTrue(SessionState().previewEnabled)
    }

    @Test
    fun `the setting survives every state change that is not about the setting`() {
        // `copy` is how the session writes every other fact — phase, stats,
        // resolution, battery. None of them may reset the user's choice. This is
        // the failure that made the old flag snap back: the session copied a
        // fresh default over it.
        val off = SessionState(previewEnabled = false)
        val afterStats = off.copy(streamFps = 29.97, thermal = "light", batteryPercent = 71)
        val afterPhase = afterStats.copy(phase = SessionState.Phase.STREAMING, actualFps = 30)

        assertFalse(afterStats.previewEnabled)
        assertFalse(afterPhase.previewEnabled)
    }

    @Test
    fun `the viewfinder is independent of the screen mode the desktop asks for`() {
        // Two different things that were easy to conflate: `set_screen` is a
        // desktop command about the panel (dim or black), while this setting is
        // the phone's own viewfinder output. A black screen with the viewfinder on
        // is a legitimate combination — the panel is dark but the app is drawing —
        // and neither may imply the other.
        val blackScreenWithPreview = SessionState(screenMode = "black", previewEnabled = true)
        val keepOnWithoutPreview = SessionState(screenMode = "keepOn", previewEnabled = false)

        assertTrue(blackScreenWithPreview.previewEnabled)
        assertFalse(keepOnWithoutPreview.previewEnabled)
        assertEquals("keepOn", keepOnWithoutPreview.appliedScreenMode())
    }

    @Test
    fun `a stream's own state object carries the setting through the stats tick`() {
        // The stats tick rewrites the whole state object once per second. If any
        // of those writes constructed a fresh SessionState instead of copying, the
        // switch would flip back on the next tick; this asserts the shape the
        // session relies on.
        var state = SessionState(previewEnabled = false)
        repeat(3) { tick ->
            state = state.copy(streamFps = 30.0 - tick, droppedPercent = 0.0)
        }
        assertFalse(state.previewEnabled)
    }
}
