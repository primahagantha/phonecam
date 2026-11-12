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

/**
 * A failure, in the product's voice.
 *
 * [com.phonecam.camera.session.SessionState.error] is already user-facing prose
 * (PRD §38, Rule 6), so it is rendered as-is: calm sentences, no exception class
 * names, no error codes, no red "ERROR" shouting. The recovery affordance is a
 * plain text action next to the sentence it belongs to, because the next thing a
 * user wants after "couldn't find your PC" is the button that tries again.
 *
 * The error border is the error hue rather than a neutral hairline, so the one
 * surface on the screen that means "this is broken" is the one surface that looks
 * different. It announces itself politely for the same reason: a failure that
 * appears while the user is looking at their PC has to reach a screen reader
 * without being requested.
 */
@Composable
fun ErrorCard(
    message: String,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
    secondaryLabel: String? = null,
    onSecondary: (() -> Unit)? = null,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(PhoneCamShapes.Panel)
            .background(MaterialTheme.colorScheme.errorContainer)
            .border(1.dp, MaterialTheme.colorScheme.error, PhoneCamShapes.Panel)
            .padding(16.dp)
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            text = stringResource(R.string.banner_error_title),
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onErrorContainer,
        )
        Text(
            text = message,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onErrorContainer,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            // The actions take the container's own on-colour rather than the
            // app accent. A TextButton defaults to `primary`, which would put the
            // green "Try again" on the red failure banner: a second colour on a
            // surface that is already saying something with colour.
            TextButton(
                onClick = onRetry,
                colors = ButtonDefaults.textButtonColors(
                    contentColor = MaterialTheme.colorScheme.onErrorContainer,
                ),
            ) {
                Text(stringResource(R.string.action_retry))
            }
            if (secondaryLabel != null && onSecondary != null) {
                TextButton(
                    onClick = onSecondary,
                    colors = ButtonDefaults.textButtonColors(
                        contentColor = MaterialTheme.colorScheme.onErrorContainer,
                    ),
                ) {
                    Text(secondaryLabel)
                }
            }
        }
    }
}
