package com.phonecam.camera.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.infoContainer
import com.phonecam.camera.ui.theme.onInfoContainer

/**
 * An inline, non-blocking information banner.
 *
 * This is where [com.phonecam.camera.session.SessionState.notice] goes. A notice
 * means "your request was substituted with something this phone can do", which is
 * normal operation, not a failure (PRD §13) — so it is shown inside the page with
 * a dismiss action rather than as a dialog or a toast that would interrupt a
 * stream the user is already watching.
 *
 * The info container is blue, deliberately not the warning amber: a substitution
 * is the system working, while the advisory banner is the phone getting hot. Two
 * conditions, two colours, and neither is the accent.
 *
 * [title] is the notice's own heading. Most notices are substitutions and keep
 * the default; the session passes an accurate one when the notice says something
 * else (a pairing approval is a notice, but not a substitution), so the heading
 * always matches the body without inventing a second banner colour.
 */
@Composable
fun InfoBanner(
    text: String,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    title: String = stringResource(R.string.banner_notice_title),
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(PhoneCamShapes.Panel)
            .background(MaterialTheme.colorScheme.infoContainer)
            .border(1.dp, MaterialTheme.colorScheme.infoContainer, PhoneCamShapes.Panel)
            .padding(start = 16.dp, end = 8.dp, top = 12.dp, bottom = 12.dp)
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = title,
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onInfoContainer,
            )
            Text(
                text = text,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onInfoContainer,
            )
        }
        // Same reason as the error banner: a TextButton defaults to the accent,
        // and the green "Dismiss" would fight the blue container it sits on.
        TextButton(
            onClick = onDismiss,
            colors = ButtonDefaults.textButtonColors(
                contentColor = MaterialTheme.colorScheme.onInfoContainer,
            ),
        ) {
            Text(stringResource(R.string.action_dismiss))
        }
    }
}
