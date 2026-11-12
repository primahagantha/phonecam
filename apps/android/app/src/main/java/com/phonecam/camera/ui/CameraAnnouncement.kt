package com.phonecam.camera.ui

import com.phonecam.camera.protocol.CameraInfo
import com.phonecam.camera.session.SessionState

/**
 * The camera the user just switched to, or `null` when they switched nothing.
 *
 * The "Switched to <camera>" toast keys on this. It used to key on
 * [SessionState.activeCamera] changing, and that field changes for reasons that
 * are not a switch: a cold session starts with no cameras and no selection, so
 * when the async capability probe lands `activeCamera` goes null → the first
 * camera, and the toast announced "Switched to Back camera" for a switch the
 * user never made. The same is true when a connection probes the cameras
 * (`sendCapabilities`) or when the list is re-read.
 *
 * What *is* a switch is a change to the selection the user asked for, which the
 * session records in [SessionState.selectedCameraId] — set by `selectCamera`
 * (desktop `set_camera`), `selectCameraLocally` (the phone's own picker) and the
 * pending-pick path that applies a choice made before the probe landed. The only
 * way that field changes without the user is `withCameras` falling back to the
 * first camera because the selected one physically disappeared from the list,
 * which cannot happen to a static hardware list.
 *
 * There must also have been a selection *before*: going from no selection to the
 * first camera is the probe landing, not a switch, which is exactly the cold
 * launch this rule exists to keep quiet.
 *
 * Kept pure and here rather than inline in the view model because a live session
 * — and, more to the point, a cold launch's probe — is not available to a unit
 * test, while the decision that was wrong is. It lives under `ui/` rather than
 * beside the session rules ([SessionState.connectedTransport] and friends)
 * because the session layer is owned by another change in flight; the rule itself
 * reads only [SessionState] and would move cleanly if that changes.
 */
fun cameraSwitchedTo(before: SessionState, after: SessionState): CameraInfo? {
    val previous = before.selectedCameraId ?: return null
    val selected = after.selectedCameraId ?: return null
    if (selected == previous) return null
    return after.activeCamera
}
