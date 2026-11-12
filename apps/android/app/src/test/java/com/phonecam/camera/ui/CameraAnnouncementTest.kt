package com.phonecam.camera.ui

import com.phonecam.camera.protocol.CameraInfo
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.session.withCameras
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The rule that decides when "Switched to <camera>" is worth saying.
 *
 * The toast keyed on [SessionState.activeCamera] changing, which is not the same
 * as the user switching. On a cold launch the state has no cameras and no
 * selection; when the async probe lands `activeCamera` goes null → the first
 * camera and the toast fired, telling the user they switched to a camera they
 * had never chosen. These cases pin the honest rule: a switch is a change to the
 * *selection*, and a probe that merely fills in the list is not one.
 */
class CameraAnnouncementTest {

    private fun camera(id: String, facing: String) = CameraInfo(
        id = id,
        facing = facing,
        resolutions = listOf("1280x720"),
        fps = listOf(30),
        torch = false,
        focus = true,
        zoom = false,
        exposure = true,
        whiteBalance = true,
        minZoom = 1f,
        maxZoom = 1f,
        orientationDegrees = 90,
    )

    private val back = camera("0", "back")
    private val front = camera("1", "front")

    @Test
    fun `the probe landing on a cold launch is not a switch`() {
        // The defect itself. The launch state has no cameras and no selection;
        // the view model's probe then adopts the list and `withCameras` selects
        // the first camera. `activeCamera` changed, but nobody switched.
        val before = SessionState()
        val after = before.withCameras(listOf(back, front))

        assertEquals(back, after.activeCamera)
        assertNull(cameraSwitchedTo(before, after))
    }

    @Test
    fun `a connection probing the cameras is not a switch`() {
        // `sendCapabilities` adopts the session's own list on connect; the user
        // did not touch the picker, so nothing is announced.
        val before = SessionState()
        val after = before.withCameras(listOf(back, front))

        assertNull(cameraSwitchedTo(before, after))
    }

    @Test
    fun `a real selection change is a switch`() {
        val before = SessionState(capabilities = listOf(back, front), selectedCameraId = "0")
        val after = before.copy(selectedCameraId = "1")

        assertEquals(front, cameraSwitchedTo(before, after))
    }

    @Test
    fun `re-selecting the camera already in use is not a switch`() {
        val before = SessionState(capabilities = listOf(back, front), selectedCameraId = "1")
        val after = before.copy(selectedCameraId = "1")

        assertNull(cameraSwitchedTo(before, after))
    }

    @Test
    fun `a state that never selected a camera is not a switch`() {
        // No selection means nothing to announce, whatever the list holds.
        val after = SessionState(capabilities = listOf(back, front))

        assertNull(cameraSwitchedTo(SessionState(), after))
    }

    @Test
    fun `a selection change with no resolvable camera is not a switch`() {
        // Defensive: if the selected id is not in the list there is no name to
        // say, and saying nothing beats naming the wrong camera.
        val before = SessionState(capabilities = listOf(back), selectedCameraId = "0")
        val after = before.copy(selectedCameraId = "gone")

        assertNull(cameraSwitchedTo(before, after))
    }
}
