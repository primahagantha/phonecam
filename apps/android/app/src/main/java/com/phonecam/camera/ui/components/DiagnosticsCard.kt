package com.phonecam.camera.ui.components

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R

/**
 * Diagnostics (PRD §62, §63).
 *
 * The log shown here is already redacted: `protocol.Log` strips secrets and
 * tokens before a line is ever stored, so this panel can be read aloud to support
 * without leaking the credential that authorises this phone. The fingerprint is
 * the `sha256(secret)[0:8]` correlator the protocol allows (§63), never the
 * secret itself.
 *
 * This is the one block on the screen with no panel around it. Everything above
 * it is something the user may act on; this is reference material they read once
 * and copy into a bug report, so it sits behind a hairline with no elevation
 * rather than competing with the cards. The log itself is the exception, and gets
 * a recessed well because it is a surface inside a surface.
 */
@Composable
fun DiagnosticsCard(
    deviceId: String,
    deviceName: String,
    secretFingerprint: String?,
    encoderLabel: String,
    cameraCount: Int,
    /**
     * Whether the camera probe has answered yet.
     *
     * `cameraCount` is 0 both before the probe and (in principle) on a phone with
     * no cameras; without this flag the row would claim "Cameras found: 0" before
     * the phone had looked.
     */
    camerasProbed: Boolean,
    appVersion: String,
    logLines: List<String>,
    onRefreshCapabilities: () -> Unit,
    onClearLog: () -> Unit,
    /**
     * Builds the full report, log plus context.
     *
     * Supplied by the caller rather than assembled here because it needs the
     * live session state, and this card deliberately renders from the values it
     * is handed.
     */
    buildReport: () -> String,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    var showLog by rememberSaveable { mutableStateOf(false) }
    var copied by rememberSaveable { mutableStateOf(false) }
    var reportCopied by rememberSaveable { mutableStateOf(false) }

    PlainSection(title = stringResource(R.string.section_diagnostics), modifier = modifier) {
        LabeledValue(
            label = stringResource(R.string.diagnostics_device_id),
            value = deviceId,
        )
        LabeledValue(
            label = stringResource(R.string.diagnostics_device_name),
            value = deviceName,
        )
        LabeledValue(
            label = stringResource(R.string.diagnostics_secret),
            value = secretFingerprint ?: stringResource(R.string.diagnostics_secret_none),
        )
        LabeledValue(
            label = stringResource(R.string.diagnostics_encoder),
            value = encoderLabel,
        )
        LabeledValue(
            label = stringResource(R.string.diagnostics_cameras),
            value = if (camerasProbed) {
                cameraCount.toString()
            } else {
                stringResource(R.string.diagnostics_not_detected)
            },
        )
        LabeledValue(
            label = stringResource(R.string.diagnostics_app_version),
            value = appVersion,
        )

        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            TextButton(onClick = onRefreshCapabilities) {
                Text(stringResource(R.string.action_refresh_capabilities))
            }
            TextButton(onClick = { showLog = !showLog }) {
                Text(
                    stringResource(
                        if (showLog) R.string.action_hide_log else R.string.action_show_log,
                    ),
                )
            }
        }

        if (showLog) {
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                TextButton(
                    onClick = {
                        copyToClipboard(context, logLines)
                        copied = true
                    },
                    // Copying an empty log puts an empty string on the clipboard
                    // and then says it worked, so the action waits for content.
                    enabled = logLines.isNotEmpty(),
                ) {
                    Text(stringResource(R.string.action_copy_log))
                }
                TextButton(onClick = onClearLog, enabled = logLines.isNotEmpty()) {
                    Text(stringResource(R.string.action_clear_log))
                }
            }

            // The report and the log are different things, so they get different
            // buttons. Copying the log gives a bare list of lines that has to be
            // paired with questions about which phone and which settings; the
            // report carries that context with it, which is what makes it
            // possible to act on without a round trip.
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                TextButton(
                    onClick = {
                        copyToClipboard(context, listOf(buildReport()))
                        reportCopied = true
                    },
                ) {
                    Text(stringResource(R.string.action_copy_report))
                }
                TextButton(onClick = { shareReport(context, buildReport()) }) {
                    Text(stringResource(R.string.action_send_report))
                }
            }
            if (reportCopied) {
                Text(
                    text = stringResource(R.string.diagnostics_report_copied),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.primary,
                )
            }
            if (copied) {
                Text(
                    text = stringResource(R.string.diagnostics_copied),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.primary,
                )
            }
            val scroll = rememberScrollState()
            InsetWell(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 260.dp)
                    .verticalScroll(scroll),
            ) {
                if (logLines.isEmpty()) {
                    Text(
                        text = stringResource(R.string.diagnostics_log_empty),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                } else {
                    logLines.forEach { line ->
                        Text(
                            text = line,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

/**
 * Hands the report to whatever the user wants to send it with.
 *
 * A share sheet rather than a fixed destination: the report is text, and whether
 * it goes to a chat, an email or a note is the user's business. `EXTRA_TEXT`
 * rather than a file, because a file needs a provider and a permission and the
 * report is small enough to travel inline.
 */
private fun shareReport(context: Context, report: String) {
    val intent = Intent(Intent.ACTION_SEND).apply {
        type = "text/plain"
        putExtra(Intent.EXTRA_SUBJECT, context.getString(R.string.report_subject))
        putExtra(Intent.EXTRA_TEXT, report)
    }
    val chooser = Intent.createChooser(intent, context.getString(R.string.action_send_report))
    // Started from a composable's click handler rather than from an activity, so
    // the chooser needs its own task on some platforms.
    chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    runCatching { context.startActivity(chooser) }
}

private fun copyToClipboard(context: Context, lines: List<String>) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
    clipboard.setPrimaryClip(ClipData.newPlainText("PhoneCam log", lines.joinToString("\n")))
}
