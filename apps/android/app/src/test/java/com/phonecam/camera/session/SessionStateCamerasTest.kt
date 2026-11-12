package com.phonecam.camera.session

import com.phonecam.camera.protocol.CameraInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The rule that keeps the selected camera in step with the camera list.
 *
 * The phone learns its cameras from three probes that run at different times —
 * the view model's, the service's and the session's — and every one of them ends
 * in [withCameras]. Getting this wrong is what produced the "Camera in use:
 * Unknown" report: `capabilities` was filled in but `selectedCameraId` was not,
 * and [SessionState.activeCamera] is a lookup by that id, so a phone with a
 * perfectly good camera appeared to have none.
 */
class SessionStateCamerasTest {

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
    fun `a phone with no session yet still has a camera in use`() {
        val state = SessionState().withCameras(listOf(back, front))

        assertEquals(listOf("0", "1"), state.capabilities.map { it.id })
        assertEquals("0", state.selectedCameraId)
        assertEquals(back, state.activeCamera)
    }

    @Test
    fun `a refresh keeps the camera the user chose`() {
        val before = SessionState(capabilities = listOf(back, front), selectedCameraId = "1")
        val after = before.withCameras(listOf(back, front))

        assertEquals("1", after.selectedCameraId)
        assertEquals(front, after.activeCamera)
    }

    @Test
    fun `a refresh follows the list when the chosen camera is gone`() {
        val before = SessionState(capabilities = listOf(back, front), selectedCameraId = "1")
        val after = before.withCameras(listOf(back))

        assertEquals("0", after.selectedCameraId)
        assertEquals(back, after.activeCamera)
    }

    @Test
    fun `a probe that found nothing does not erase what an earlier one found`() {
        val before = SessionState(capabilities = listOf(back, front), selectedCameraId = "1")

        assertEquals(before, before.withCameras(emptyList()))
    }

    /** The defect itself, so a regression cannot pass unnoticed. */
    @Test
    fun `filling in the list without the selection leaves no camera in use`() {
        val listOnly = SessionState().copy(capabilities = listOf(back, front))

        assertNull(listOnly.selectedCameraId)
        assertNull(listOnly.activeCamera)
    }
}
