package com.phonecam.camera.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.widget.Toast
import com.phonecam.camera.R
import com.phonecam.camera.protocol.Log
import com.phonecam.camera.session.SessionIntents
import com.phonecam.camera.session.SessionState

/**
 * Session control from outside the activity (a notification action, an
 * automation app, or `adb shell am broadcast`).
 *
 * `ACTION_STOP_STREAM` deserves an explanation, because the protocol cannot
 * express it as a message. `start_stream` and `stop_stream` only travel
 * desktop→phone (docs/protocol.md §6.2); §6.1 has no phone→desktop counterpart.
 * The one thing the phone *can* do that genuinely stops the video is close the
 * video socket (§7.2), so that is what this does: it ends the session, and says
 * so rather than implying it sent a command the PC would honour.
 */
class StreamActionReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            SessionIntents.ACTION_STOP_STREAM -> {
                val wasStreaming =
                    CameraStreamService.sessionState.value.phase == SessionState.Phase.STREAMING
                SessionIntents.dispatch(context, SessionIntents.stop(context))
                Toast.makeText(
                    context,
                    if (wasStreaming) R.string.toast_stream_stopped else R.string.toast_session_stopped,
                    Toast.LENGTH_SHORT,
                ).show()
            }

            SessionIntents.ACTION_START -> {
                SessionIntents.dispatch(context, SessionIntents.start(context))
            }

            else -> Log.d("stream receiver ignoring ${intent.action}")
        }
    }
}
