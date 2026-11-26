package com.phonecam.camera.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.PhoneCamStudioTheme

/**
 * Compact persistent studio top bar shared across Portrait and Landscape.
 *
 * Replaces the old bottom StatusStrip stacked above NavigationBar, reclaiming
 * vertical workspace (especially in landscape) while keeping live session state
 * and the Start / Stop control permanently one tap away.
 */
@Composable
fun StudioTopBar(
    state: SessionState,
    isRunning: Boolean,
    isLandscape: Boolean,
    onStart: () -> Unit,
    onStop: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val detail: String? = when (state.phase) {
        SessionState.Phase.STREAMING ->
            state.serverName
                ?.let { stringResource(R.string.overlay_streaming_title, it) }
                ?: stringResource(R.string.overlay_streaming_title_unknown)

        SessionState.Phase.CONNECTED ->
            state.serverName?.let { stringResource(R.string.notification_text_connected, it) }

        SessionState.Phase.DISCOVERING,
        SessionState.Phase.CONNECTING,
        SessionState.Phase.RECONNECTING,
        -> stringResource(R.string.notification_text_connecting)

        else -> null
    }

    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface)
            .statusBarsPadding(),
    ) {
        if (isLandscape) {
            // Compact 44dp landscape studio bar: status pill + PC target + crisp Start/Stop.
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 44.dp)
                    .padding(horizontal = 14.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                PhasePill(state = state, compact = true)
                Text(
                    text = detail ?: stringResource(R.string.app_name),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                SessionActionButton(
                    isRunning = isRunning,
                    compact = false,
                    onStart = onStart,
                    onStop = onStop,
                )
            }
        } else {
            // Clean Portrait studio header: app title + live status pill + compact Stop/Start.
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 52.dp)
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(
                            text = stringResource(R.string.app_name),
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.SemiBold,
                            color = MaterialTheme.colorScheme.onSurface,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        PhasePill(state = state, compact = true)
                    }
                    if (!detail.isNullOrBlank()) {
                        Text(
                            text = detail,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                SessionActionButton(
                    isRunning = isRunning,
                    compact = true,
                    onStart = onStart,
                    onStop = onStop,
                )
            }
        }
        Hairline()
    }
}

/**
 * Semantically coloured Start / Stop studio button.
 *
 * - Running (Stop): subtle dark-red danger tint (`#2A1418`), `#7F1D1D` border,
 *   `#FCA5A5` icon & label — communicates destructive stop without bright neon.
 * - Stopped (Start): Studio Blue (`#2563EB`) primary CTA with `#F8FAFC` text.
 */
@Composable
fun SessionActionButton(
    isRunning: Boolean,
    compact: Boolean,
    onStart: () -> Unit,
    onStop: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val studio = PhoneCamStudioTheme.colors
    val contentPadding = if (compact) {
        PaddingValues(horizontal = 12.dp, vertical = 6.dp)
    } else {
        PaddingValues(horizontal = 14.dp, vertical = 6.dp)
    }

    if (isRunning) {
        OutlinedButton(
            onClick = onStop,
            modifier = modifier.heightIn(min = 34.dp),
            shape = PhoneCamShapes.Chip,
            border = BorderStroke(1.dp, studio.dangerBorder),
            colors = ButtonDefaults.outlinedButtonColors(
                containerColor = studio.dangerSurface,
                contentColor = studio.dangerText,
            ),
            contentPadding = contentPadding,
        ) {
            Icon(
                imageVector = Icons.Filled.Stop,
                contentDescription = null,
                modifier = Modifier.size(16.dp),
            )
            Spacer(Modifier.width(5.dp))
            Text(
                text = stringResource(
                    if (compact) R.string.action_stop_short else R.string.action_stop_session,
                ),
                style = MaterialTheme.typography.labelLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    } else {
        Button(
            onClick = onStart,
            modifier = modifier.heightIn(min = 34.dp),
            shape = PhoneCamShapes.Chip,
            colors = ButtonDefaults.buttonColors(
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
            ),
            contentPadding = contentPadding,
        ) {
            Icon(
                imageVector = Icons.Filled.PlayArrow,
                contentDescription = null,
                modifier = Modifier.size(16.dp),
            )
            Spacer(Modifier.width(5.dp))
            Text(
                text = stringResource(
                    if (compact) R.string.action_start_short else R.string.action_start_session,
                ),
                style = MaterialTheme.typography.labelLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}
