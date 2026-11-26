package com.phonecam.camera.ui.components

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.PhoneCamStudioTheme

/**
 * Diagnostics card: device identifiers, encoder telemetry, redacted session log,
 * and bug report export actions.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun DiagnosticsCard(
    deviceId: String,
    deviceName: String,
    secretFingerprint: String?,
    encoderLabel: String,
    cameraCount: Int,
    camerasProbed: Boolean,
    appVersion: String,
    logLines: List<String>,
    onRefreshCapabilities: () -> Unit,
    onClearLog: () -> Unit,
    buildReport: () -> String,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val studio = PhoneCamStudioTheme.colors
    var showLog by rememberSaveable { mutableStateOf(false) }
    var copied by rememberSaveable { mutableStateOf(false) }
    var reportCopied by rememberSaveable { mutableStateOf(false) }

    SectionCard(title = stringResource(R.string.section_diagnostics), modifier = modifier) {
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

        Hairline()

        FlowRow(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            OutlinedButton(
                onClick = onRefreshCapabilities,
                shape = PhoneCamShapes.Chip,
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 7.dp),
            ) {
                Text(stringResource(R.string.action_refresh_capabilities))
            }
            OutlinedButton(
                onClick = { showLog = !showLog },
                shape = PhoneCamShapes.Chip,
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 7.dp),
            ) {
                Text(
                    stringResource(
                        if (showLog) R.string.action_hide_log else R.string.action_show_log,
                    ),
                )
            }
        }

        if (showLog) {
            FlowRow(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                TextButton(
                    onClick = {
                        copyToClipboard(context, logLines)
                        copied = true
                    },
                    enabled = logLines.isNotEmpty(),
                    shape = PhoneCamShapes.Chip,
                ) {
                    Text(stringResource(R.string.action_copy_log))
                }
                TextButton(
                    onClick = onClearLog,
                    enabled = logLines.isNotEmpty(),
                    shape = PhoneCamShapes.Chip,
                ) {
                    Text(stringResource(R.string.action_clear_log))
                }
            }

            FlowRow(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                TextButton(
                    onClick = {
                        copyToClipboard(context, listOf(buildReport()))
                        reportCopied = true
                    },
                    shape = PhoneCamShapes.Chip,
                ) {
                    Text(stringResource(R.string.action_copy_report))
                }
                TextButton(
                    onClick = { shareReport(context, buildReport()) },
                    shape = PhoneCamShapes.Chip,
                ) {
                    Text(stringResource(R.string.action_send_report))
                }
                TextButton(
                    onClick = {
                        val reportText = buildReport()
                        copyToClipboard(context, listOf(reportText))
                        val intent = com.phonecam.camera.protocol.CrashReporter.buildGitHubIssueIntent(
                            context = context,
                            title = "[Bug] Issue report from ${android.os.Build.MODEL}",
                            bodyMarkdown = "### Description\n(Describe what happened)\n\n### System Diagnostics\n```\n$reportText\n```",
                        )
                        runCatching { context.startActivity(intent) }
                    },
                    shape = PhoneCamShapes.Chip,
                ) {
                    Text(stringResource(R.string.action_report_github))
                }
            }
            if (reportCopied) {
                Text(
                    text = stringResource(R.string.diagnostics_report_copied),
                    style = MaterialTheme.typography.bodySmall,
                    color = studio.live,
                )
            }
            if (copied) {
                Text(
                    text = stringResource(R.string.diagnostics_copied),
                    style = MaterialTheme.typography.bodySmall,
                    color = studio.live,
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
                            style = MaterialTheme.typography.bodySmall.copy(
                                fontFamily = FontFamily.Monospace,
                            ),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

private fun shareReport(context: Context, report: String) {
    val intent = Intent(Intent.ACTION_SEND).apply {
        type = "text/plain"
        putExtra(Intent.EXTRA_SUBJECT, context.getString(R.string.report_subject))
        putExtra(Intent.EXTRA_TEXT, report)
    }
    val chooser = Intent.createChooser(intent, context.getString(R.string.action_send_report))
    chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    runCatching { context.startActivity(chooser) }
}

private fun copyToClipboard(context: Context, lines: List<String>) {
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
    clipboard.setPrimaryClip(
        ClipData.newPlainText(context.getString(R.string.clipboard_log), lines.joinToString("\n")),
    )
}
