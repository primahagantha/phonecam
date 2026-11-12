package com.phonecam.camera.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.session.overlayHintVisible
import com.phonecam.camera.ui.theme.PhoneCamShapes

/**
 * The strip pinned to the bottom while streaming.
 *
 * While a stream is live the phone is usually face-down next to the keyboard, so
 * the only things worth the screen are: is it still going, where is it going, and
 * how do I stop it. The strip floats over the page rather than replacing it, so
 * the controls stay one tap away.
 *
 * It uses the theme's `inverseSurface` rather than a hardcoded translucent black.
 * The theme value is a real surface, so the text on it is a verified contrast pair
 * in both schemes instead of depending on whatever happens to be scrolled behind
 * it, and "inverse" is the token Material reserves for exactly this: a surface
 * that has to read as being above the content.
 */
@Composable
fun StreamingOverlay(
    state: SessionState,
    onStop: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val title = state.serverName
        ?.let { stringResource(R.string.overlay_streaming_title, it) }
        ?: stringResource(R.string.overlay_streaming_title_unknown)

    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(12.dp)
            .clip(PhoneCamShapes.Panel)
            .background(MaterialTheme.colorScheme.inverseSurface)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, PhoneCamShapes.Panel)
            .padding(start = 16.dp, end = 8.dp, top = 10.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = title,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.inverseOnSurface,
            )
            // The hint describes the tap that uncovers the blackout surface, so
            // it is only shown in `screen_black`, the one mode where the overlay
            // is actually hidden behind a tap. Everywhere else the controls are
            // already visible and the sentence promised an interaction that did
            // not exist.
            if (state.overlayHintVisible()) {
                Text(
                    text = stringResource(R.string.overlay_hint),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.inverseOnSurface,
                )
            }
        }
        TextButton(onClick = onStop) {
            Text(
                text = stringResource(R.string.action_stop_stream),
                color = MaterialTheme.colorScheme.inversePrimary,
            )
        }
    }
}
