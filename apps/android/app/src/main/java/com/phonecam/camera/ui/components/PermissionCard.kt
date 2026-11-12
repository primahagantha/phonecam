package com.phonecam.camera.ui.components

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import com.phonecam.camera.R

/**
 * What the camera permission costs the user, in one sentence, before asking.
 *
 * The grant button is filled and full width: while the permission is missing this
 * is the only thing the user can usefully do, so it is the primary action on the
 * screen.
 */
@Composable
fun CameraPermissionCard(
    onRequest: () -> Unit,
    modifier: Modifier = Modifier,
) {
    SectionCard(title = stringResource(R.string.permission_title), modifier = modifier) {
        Text(
            text = stringResource(R.string.permission_rationale),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button(onClick = onRequest, modifier = Modifier.fillMaxWidth()) {
            Text(stringResource(R.string.permission_action_grant))
        }
    }
}

/**
 * Shown after a permanent denial.
 *
 * The platform will no longer display a dialog at this point, so the only honest
 * affordance is a route to Settings. It is deliberately not a red error: the app
 * is not broken, it is waiting for one switch. The action is outlined rather than
 * filled, because it leaves the app and the user should expect that.
 */
@Composable
fun PermissionDeniedCard(
    onOpenSettings: () -> Unit,
    modifier: Modifier = Modifier,
) {
    SectionCard(title = stringResource(R.string.permission_denied_title), modifier = modifier) {
        Text(
            text = stringResource(R.string.permission_denied_body),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        TextButton(onClick = onOpenSettings) {
            Text(stringResource(R.string.permission_action_settings))
        }
    }
}

/**
 * A quiet line when notifications are off.
 *
 * Notifications are how the streaming status stays visible outside the app, but
 * refusing them does not stop streaming, so this must not be an error and must
 * not nag (PRD §39).
 */
@Composable
fun NotificationPermissionNote(
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    // An explicit heading: the banner's default is the substitution title
    // ("Using a supported setting"), which describes a substituted request, not
    // a refused permission. Left to the default the screen announced a
    // substitution that never happened. "Heads up" is the same heading the
    // advisory banner uses for a condition the user cannot dismiss.
    InfoBanner(
        text = stringResource(R.string.notification_permission_warning),
        onDismiss = onDismiss,
        modifier = modifier,
        title = stringResource(R.string.banner_local_title),
    )
}
