package com.phonecam.camera.session

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat
import com.phonecam.camera.protocol.Proto
import com.phonecam.camera.service.CameraStreamService

/**
 * The intent vocabulary the UI and the foreground service share.
 *
 * The service is the only component that owns a [StreamSession], so the UI never
 * touches the camera, the sockets, or the encoder itself: it sends one of these
 * intents and renders [CameraStreamService.sessionState]. One owner per session
 * means there is no second copy of the state to keep in sync, and the session
 * survives the activity being destroyed.
 *
 * The protocol is desktop-driven: `start_stream` / `stop_stream` are
 * desktop→phone messages (docs/protocol.md §6.2) and the phone has no equivalent
 * to send. Nothing here pretends otherwise — these intents control the *session*
 * (connect, disconnect, search, retry), not the stream.
 */
object SessionIntents {

    const val ACTION_START = "com.phonecam.camera.action.START_SESSION"
    const val ACTION_STOP = "com.phonecam.camera.action.STOP_SESSION"
    const val ACTION_SEARCH = "com.phonecam.camera.action.SEARCH"
    const val ACTION_DISCONNECT = "com.phonecam.camera.action.DISCONNECT"
    const val ACTION_CONNECT_DISCOVERED = "com.phonecam.camera.action.CONNECT_DISCOVERED"
    const val ACTION_CONNECT_MANUAL = "com.phonecam.camera.action.CONNECT_MANUAL"
    const val ACTION_CONNECT_TOKEN = "com.phonecam.camera.action.CONNECT_TOKEN"
    const val ACTION_REFRESH_CAPABILITIES = "com.phonecam.camera.action.REFRESH_CAPABILITIES"
    const val ACTION_SELECT_CAMERA = "com.phonecam.camera.action.SELECT_CAMERA"
    const val ACTION_ACKNOWLEDGE_NOTICE = "com.phonecam.camera.action.ACKNOWLEDGE_NOTICE"

    /**
     * Turns this phone's own viewfinder on or off (a phone-owned setting).
     *
     * It belongs here rather than in the view model because the capture session
     * that gains or loses the preview output is owned by the service: a flag held
     * only by the activity would be lost the moment the user went to another app,
     * and the service would rebuild the camera with the wrong output set.
     */
    const val ACTION_SET_PREVIEW = "com.phonecam.camera.action.SET_PREVIEW"

    /** Sent by [com.phonecam.camera.service.StreamActionReceiver] from outside the activity. */
    const val ACTION_STOP_STREAM = "com.phonecam.camera.action.STOP_STREAM"

    const val EXTRA_HOST = "host"
    const val EXTRA_PORT = "port"
    const val EXTRA_TOKEN = "token"
    const val EXTRA_SERVER_ID = "serverId"
    const val EXTRA_CAMERA = "camera"
    const val EXTRA_STOP_REASON = "stopReason"
    const val EXTRA_PREVIEW_ENABLED = "previewEnabled"

    const val STOP_REASON_USER = "user"
    const val STOP_REASON_SYSTEM = "system"

    /** The desktop core's TCP port; shared with the protocol rather than re-typed. */
    const val DEFAULT_PORT = Proto.SERVICE_PORT

    private fun intent(context: Context, action: String, block: Intent.() -> Unit = {}): Intent =
        Intent(context, CameraStreamService::class.java).setAction(action).apply(block)

    fun start(context: Context): Intent = intent(context, ACTION_START)

    fun stop(context: Context): Intent =
        intent(context, ACTION_STOP) { putExtra(EXTRA_STOP_REASON, STOP_REASON_USER) }

    fun search(context: Context): Intent = intent(context, ACTION_SEARCH)

    fun disconnect(context: Context): Intent = intent(context, ACTION_DISCONNECT)

    fun connectDiscovered(context: Context, serverId: String, host: String, port: Int): Intent =
        intent(context, ACTION_CONNECT_DISCOVERED) {
            putExtra(EXTRA_SERVER_ID, serverId)
            putExtra(EXTRA_HOST, host)
            putExtra(EXTRA_PORT, port)
        }

    fun connectManual(context: Context, host: String, port: Int): Intent =
        intent(context, ACTION_CONNECT_MANUAL) {
            putExtra(EXTRA_HOST, host)
            putExtra(EXTRA_PORT, port)
        }

    fun connectWithToken(context: Context, host: String, port: Int, token: String): Intent =
        intent(context, ACTION_CONNECT_TOKEN) {
            putExtra(EXTRA_HOST, host)
            putExtra(EXTRA_PORT, port)
            putExtra(EXTRA_TOKEN, token)
        }

    fun refreshCapabilities(context: Context): Intent = intent(context, ACTION_REFRESH_CAPABILITIES)

    /** [facingOrId] is a facing (`"back"` / `"front"`) or a camera id (§6.2 vocabulary). */
    fun selectCamera(context: Context, facingOrId: String): Intent =
        intent(context, ACTION_SELECT_CAMERA) { putExtra(EXTRA_CAMERA, facingOrId) }

    fun acknowledgeNotice(context: Context): Intent = intent(context, ACTION_ACKNOWLEDGE_NOTICE)

    fun setPreviewEnabled(context: Context, enabled: Boolean): Intent =
        intent(context, ACTION_SET_PREVIEW) { putExtra(EXTRA_PREVIEW_ENABLED, enabled) }

    fun stopStream(context: Context): Intent = intent(context, ACTION_STOP_STREAM)

    /**
     * Delivers [intent] to the service.
     *
     * `startForegroundService` is only used when the camera permission is already
     * granted: a camera-typed foreground service cannot start without it, and the
     * platform treats "started as foreground but never called startForeground" as
     * a crash. With the permission missing the caller is the UI, which is in the
     * foreground, so a plain `startService` is both legal and honest — the service
     * then reports the missing permission instead of dying.
     */
    fun dispatch(context: Context, intent: Intent) {
        val app = context.applicationContext
        val canRunCameraService = ContextCompat.checkSelfPermission(
            app,
            Manifest.permission.CAMERA,
        ) == PackageManager.PERMISSION_GRANTED

        if (canRunCameraService && Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            app.startForegroundService(intent)
        } else {
            app.startService(intent)
        }
    }
}
