package com.phonecam.camera.ui.components

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.OpenInNew
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.update.GitHubUpdateChecker
import com.phonecam.camera.update.UpdateCheckState
import com.phonecam.camera.update.UpdateErrorReason

/**
 * Software Updates card for the Diagnostics screen.
 *
 * Strictly user-initiated: PhoneCam never polls GitHub or any external server
 * in the background. Tapping "Check for Updates" performs a single HTTPS GET
 * to GitHub Releases and renders the latest version and changelog inline.
 */
@Composable
fun UpdateCard(
    appVersion: String,
    updateState: UpdateCheckState,
    onCheckForUpdates: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val currentDisplayVersion = GitHubUpdateChecker.formatDisplayVersion(appVersion)
    val isChecking = updateState is UpdateCheckState.Checking

    SectionCard(
        title = stringResource(R.string.section_updates),
        subtitle = stringResource(R.string.updates_subtitle),
        modifier = modifier,
    ) {
        LabeledValue(
            label = stringResource(R.string.updates_current_version),
            value = currentDisplayVersion,
        )

        when (updateState) {
            UpdateCheckState.Idle -> Unit

            UpdateCheckState.Checking -> {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .semantics { liveRegion = LiveRegionMode.Polite },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.primary,
                    )
                    Text(
                        text = stringResource(R.string.updates_checking),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            is UpdateCheckState.UpToDate -> {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .semantics { liveRegion = LiveRegionMode.Polite },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Icon(
                        imageVector = Icons.Filled.CheckCircle,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(18.dp),
                    )
                    Text(
                        text = stringResource(
                            R.string.updates_up_to_date,
                            GitHubUpdateChecker.formatDisplayVersion(updateState.latestTag),
                        ),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
            }

            is UpdateCheckState.UpdateAvailable -> {
                val release = updateState.release
                val displayTag = GitHubUpdateChecker.formatDisplayVersion(release.tagName)
                val publishedDate = GitHubUpdateChecker.formatPublishedDate(release.publishedAt)

                Hairline()

                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .semantics { liveRegion = LiveRegionMode.Polite },
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Column(modifier = Modifier.weight(1f)) {
                            Text(
                                text = stringResource(R.string.updates_available_title),
                                style = MaterialTheme.typography.titleSmall,
                                color = MaterialTheme.colorScheme.onSurface,
                            )
                            if (publishedDate.isNotEmpty()) {
                                Text(
                                    text = stringResource(R.string.updates_published_on, publishedDate),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                        StatusToken(
                            text = displayTag,
                            state = TokenState.InUse,
                        )
                    }

                    if (release.name.isNotBlank() &&
                        !release.name.equals(release.tagName, ignoreCase = true) &&
                        !release.name.equals(displayTag, ignoreCase = true)
                    ) {
                        Text(
                            text = release.name,
                            style = MaterialTheme.typography.labelLarge,
                            color = MaterialTheme.colorScheme.onSurface,
                        )
                    }

                    SectionLabel(stringResource(R.string.updates_changelog_label))

                    val changelogScroll = rememberScrollState()
                    InsetWell(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(max = 220.dp)
                            .verticalScroll(changelogScroll),
                    ) {
                        Text(
                            text = release.body.ifBlank {
                                stringResource(R.string.updates_changelog_empty)
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }

                    Button(
                        onClick = {
                            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(release.htmlUrl)).apply {
                                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                            }
                            runCatching { context.startActivity(intent) }
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.OpenInNew,
                            contentDescription = null,
                            modifier = Modifier.size(18.dp),
                        )
                        Spacer(Modifier.width(8.dp))
                        Text(stringResource(R.string.action_download_github))
                    }
                }
            }

            is UpdateCheckState.Error -> {
                val message = when (updateState.reason) {
                    UpdateErrorReason.Network -> stringResource(R.string.updates_error_network)
                    UpdateErrorReason.RateLimited -> stringResource(R.string.updates_error_rate_limited)
                    UpdateErrorReason.NoRelease -> stringResource(R.string.updates_error_no_release)
                    UpdateErrorReason.ServerError -> stringResource(
                        R.string.updates_error_server,
                        updateState.detail ?: "?",
                    )
                    UpdateErrorReason.InvalidResponse -> stringResource(R.string.updates_error_invalid_response)
                }
                Text(
                    text = message,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier
                        .fillMaxWidth()
                        .semantics { liveRegion = LiveRegionMode.Polite },
                )
            }
        }

        OutlinedButton(
            onClick = onCheckForUpdates,
            enabled = !isChecking,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(
                imageVector = Icons.Filled.Refresh,
                contentDescription = null,
                modifier = Modifier.size(18.dp),
            )
            Spacer(Modifier.width(8.dp))
            Text(stringResource(R.string.action_check_updates))
        }
    }
}
