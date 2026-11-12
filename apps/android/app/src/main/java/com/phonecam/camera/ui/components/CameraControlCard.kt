package com.phonecam.camera.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CameraFront
import androidx.compose.material.icons.filled.CameraRear
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.PhotoCamera
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.protocol.CameraInfo
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.session.appliedScreenMode
import com.phonecam.camera.session.appliedTorch

/**
 * The camera's controls, every one of them gated on [CameraInfo].
 *
 * The card is split into three blocks that answer three different questions, and
 * keeping them apart is the whole design:
 *
 *  1. *Which camera is this phone using* — the one thing the user can actually
 *     change from here, so it is a control.
 *  2. *What is the picture set to* — values the desktop owns and this phone
 *     reports. Read-only, and labelled as such.
 *  3. *What can this camera do* — capabilities. **Not** switches.
 *
 * That third block is the correction. The protocol makes the desktop the driver:
 * `set_video`, `set_image`, `set_torch` and `set_screen` are all desktop→phone
 * (docs/protocol.md §6.2). A capability such as "this camera has autofocus" is
 * therefore not a toggle, and rendering it as a disabled switch said two wrong
 * things at once: that the user could flip it, and that "has autofocus" was the
 * same kind of fact as "autofocus is on". Capabilities are stated in words; only
 * the things that genuinely have an on/off state are shown with one.
 *
 * The one exception to desktop ownership is the camera choice: the phone may pick
 * its own camera locally through [onSelectCamera], because the user is the one
 * holding the device and pointing it. The desktop is not left guessing — the
 * session reports the new camera in its next `state` message
 * (docs/protocol.md §6.3), so both ends agree without a new wire message.
 */
@Composable
fun CameraControlCard(
    state: SessionState,
    onSelectCamera: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val camera = state.activeCamera

    SectionCard(
        title = stringResource(R.string.section_camera),
        subtitle = stringResource(R.string.camera_owner_note),
        modifier = modifier,
    ) {
        if (state.capabilities.isEmpty()) {
            // The probe that fills this list is asynchronous, so this is the
            // state the card opens in on a cold start. It is a genuine empty
            // state: it names what is missing and what will fill it.
            EmptyState(stringResource(R.string.camera_loading))
            return@SectionCard
        }

        SectionLabel(stringResource(R.string.label_camera_in_use))
        CameraPicker(
            cameras = state.capabilities,
            selectedId = camera?.id,
            onSelect = onSelectCamera,
        )
    }
}

/**
 * What the desktop receives, and how the phone behaves while it does.
 *
 * The output-side settings that used to be buried in the camera card's "Picture
 * settings" block live here now: the output transform (mirror, rotation), the
 * zoom the desktop asked for, and the screen mode. The reason they are split out
 * of the camera card is that none of them is a capability of the camera — they
 * are the shape of the output, chosen by the desktop (docs/protocol.md §6.2), so
 * they belong on the screen that answers "what is the PC getting".
 *
 * The negotiation pair and the throughput numbers are not repeated here: they are
 * the [StatusCard]'s subject, and it is on this screen too.
 *
 * Nothing here is a control. The protocol makes the desktop the driver of
 * `set_video`, `set_image` and `set_screen`, so these are read-only values; the
 * hint under the title says where they are changed, because a screen full of
 * numbers with no way to act on them otherwise reads as broken.
 */
@Composable
fun OutputCard(state: SessionState, modifier: Modifier = Modifier) {
    SectionCard(
        title = stringResource(R.string.section_output),
        subtitle = stringResource(R.string.settings_note),
        modifier = modifier,
    ) {
        OutputSettings(state = state)
    }
}

/**
 * The values the desktop owns that are not already on the [StatusCard].
 *
 * Torch is the one entry here that is a real on/off state, so it is the one entry
 * that reads as one.
 */
@Composable
private fun OutputSettings(state: SessionState) {
    val context = LocalContext.current
    LabeledValue(
        label = stringResource(R.string.control_quality_mode),
        value = qualityModeLabel(context, state.qualityMode),
    )
    LabeledValue(
        label = stringResource(R.string.control_mirror),
        value = stringResource(if (state.mirror) R.string.value_yes else R.string.value_no),
    )
    LabeledValue(
        label = stringResource(R.string.control_rotation),
        value = stringResource(R.string.rotation_degrees, state.rotation),
    )
    LabeledValue(
        label = stringResource(R.string.control_zoom),
        value = stringResource(R.string.zoom_factor, state.zoom),
    )
    LabeledValue(
        label = stringResource(R.string.control_screen_mode),
        value = screenModeLabel(context, state.appliedScreenMode()),
    )
    // Torch is shown only when it is actually on. "Torch: Off" on a camera that
    // has a flash is the default and says nothing; whether the camera *has* one
    // is the capability list's job, not this block's. When the torch is lit it is
    // worth a row, because it changes what the subject sees.
    //
    // "Actually on" is a rule, not the raw field: the torch lives in the camera's
    // repeating request and dies with the stream, so the row must not outlive it.
    // See [SessionState.appliedTorch].
    if (state.appliedTorch()) {
        LabeledValue(
            label = stringResource(R.string.control_torch),
            value = stringResource(R.string.value_on),
            valueColor = MaterialTheme.colorScheme.primary,
        )
    }
}

/**
 * One labelled button per camera: a large icon plus the camera's name.
 *
 * The buttons used to be icons only, which asked the user to recognise a
 * pictogram of a camera body from a camera face and to know which one meant
 * "the one pointing at me". That is a puzzle even for someone who has used
 * phones for years, and it is worse for anyone with weaker eyesight or less
 * confidence with technology: the row read as two identical little shapes.
 *
 * Each button now carries the words that answer the question — "Kamera Depan",
 * "Kamera Belakang" — under a large icon, with the selected one filled and
 * marked "in use". The whole tile is the touch target (48dp minimum, taller
 * here), so it is easy to hit without aiming.
 *
 * The buttons are the phone's own camera switch: tapping one calls [onSelect]
 * with the camera id, and the session restarts the stream on that camera if one
 * is running. A camera the phone reported but that the session cannot currently
 * use is disabled rather than hidden, per the card's rules —
 * [cameraUnavailableReason] says which and why.
 */
@Composable
private fun CameraPicker(
    cameras: List<CameraInfo>,
    selectedId: String?,
    onSelect: (String) -> Unit,
) {
    val context = LocalContext.current
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        // Phones with three or four cameras exist, and a labelled tile is wider
        // than an icon, so the row scrolls rather than squeezing the words.
        Row(
            modifier = Modifier.horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            cameras.forEach { camera ->
                val selected = camera.id == selectedId
                val reason = cameraUnavailableReason(context, camera)
                CameraTile(
                    camera = camera,
                    selected = selected,
                    enabled = reason == null,
                    onClick = { onSelect(camera.id) },
                )
            }
        }
        // A disabled tile's explanation, in the same voice as the other gates.
        cameras.forEach { camera ->
            cameraUnavailableReason(context, camera)?.let { reason ->
                EmptyState(text = "${cameraLabel(context, camera)}: $reason")
            }
        }
    }
}

/**
 * A camera as a large, labelled tile.
 *
 * Filled/tonal means "this is the camera in use", outlined means "this is a
 * camera you could switch to", and disabled means "this camera cannot be used
 * from here" — three states a user can tell apart without reading a legend. The
 * words underneath are what make it readable at a glance; the icon alone never
 * was.
 */
@Composable
private fun CameraTile(
    camera: CameraInfo,
    selected: Boolean,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    val context = LocalContext.current
    val icon = cameraIcon(camera)
    val label = cameraLabel(context, camera)
    val description = stringResource(
        if (selected) R.string.camera_switch_selected else R.string.camera_switch_action,
        label,
    )
    val container = if (selected) {
        MaterialTheme.colorScheme.secondaryContainer
    } else {
        MaterialTheme.colorScheme.surface
    }
    val content = if (selected) {
        MaterialTheme.colorScheme.onSecondaryContainer
    } else {
        MaterialTheme.colorScheme.onSurface
    }
    // A plain clickable Surface, not an icon-toggle: the tile holds an icon and
    // two lines of text, and the toggle component would centre and clip them.
    Surface(
        onClick = onClick,
        enabled = enabled,
        shape = MaterialTheme.shapes.large,
        color = container,
        contentColor = content,
        border = if (selected) {
            null
        } else {
            BorderStroke(1.dp, MaterialTheme.colorScheme.outline)
        },
        tonalElevation = if (selected) 2.dp else 0.dp,
        modifier = Modifier
            .widthIn(min = 132.dp)
            .semantics { contentDescription = description },
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Icon(imageVector = icon, contentDescription = null, modifier = Modifier.size(40.dp))
            Text(
                text = label,
                style = MaterialTheme.typography.titleSmall,
                textAlign = TextAlign.Center,
            )
            Text(
                text = stringResource(
                    if (selected) R.string.camera_tile_in_use else R.string.camera_tile_switch,
                ),
                style = MaterialTheme.typography.bodySmall,
                color = if (selected) content else MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }
    }
}

/** The icon that stands for a camera: rear, front, or anything external. */
private fun cameraIcon(camera: CameraInfo) = when (camera.facing) {
    "front" -> Icons.Filled.CameraFront
    "back" -> Icons.Filled.CameraRear
    "external" -> Icons.Filled.Videocam
    else -> Icons.Filled.PhotoCamera
}

/**
 * What the camera can do, as statements rather than as switches.
 *
 * This is the block that used to be five disabled switches showing "Yes"/"No",
 * which implied the user could change them and conflated a capability with a
 * state. A row here answers one question: can this camera do it, yes or no.
 *
 * The leading icon is decorative: the words beside it carry the same fact, so
 * marking the icon as presentational keeps a screen reader from announcing it
 * twice.
 */
@Composable
private fun CapabilityList(camera: CameraInfo?) {
    CapabilityRow(
        label = stringResource(R.string.control_torch),
        supported = camera?.torch == true,
        reason = stringResource(R.string.reason_no_flash),
    )
    CapabilityRow(
        label = stringResource(R.string.control_focus),
        supported = camera?.focus == true,
        reason = stringResource(R.string.reason_no_focus),
    )
    CapabilityRow(
        label = stringResource(R.string.control_exposure),
        supported = camera?.exposure == true,
        reason = stringResource(R.string.reason_no_exposure),
    )
    CapabilityRow(
        label = stringResource(R.string.control_white_balance),
        supported = camera?.whiteBalance == true,
        reason = stringResource(R.string.reason_no_white_balance),
    )
    CapabilityRow(
        label = stringResource(R.string.control_zoom),
        supported = camera?.zoom == true,
        reason = stringResource(R.string.reason_no_zoom),
    )
}

@Composable
private fun CapabilityRow(label: String, supported: Boolean, reason: String) {
    val tint = if (supported) {
        MaterialTheme.colorScheme.primary
    } else {
        MaterialTheme.colorScheme.onSurfaceVariant
    }
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Icon(
                imageVector = if (supported) Icons.Filled.Check else Icons.Filled.Close,
                contentDescription = null,
                tint = tint,
                modifier = Modifier.size(18.dp),
            )
            Text(
                text = label,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.weight(1f),
            )
            Text(
                text = stringResource(
                    if (supported) R.string.capability_available else R.string.capability_unavailable,
                ),
                style = MaterialTheme.typography.bodySmall,
                color = tint,
            )
        }
        if (!supported) {
            EmptyState(text = reason)
        }
    }
}

/**
 * The raw capability probe, so a bug report can be checked against it.
 *
 * Long lists of resolutions and frame rates are tokens rather than a comma-joined
 * sentence: the set is what matters, and a wrapped list of tokens is read at a
 * glance where "1920x1080, 1280x720, 640x480" is not.
 *
 * This is reference material rather than a task, so it lives in a
 * [PlainSection]: no panel, no elevation, just a hairline and a title.
 */
@Composable
fun CameraCapabilitySection(camera: CameraInfo?, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    PlainSection(
        title = stringResource(R.string.camera_capabilities_title),
        modifier = modifier,
    ) {
        if (camera == null) {
            EmptyState(stringResource(R.string.capability_none))
            return@PlainSection
        }
        Hint(stringResource(R.string.camera_capabilities_note))

        CapabilityList(camera)

        Hairline()
        SectionLabel(stringResource(R.string.capability_resolutions))
        if (camera.resolutions.isEmpty()) {
            EmptyState(stringResource(R.string.capability_none))
        } else {
            TokenRow(
                tokens = camera.resolutions.map { resolutionLabel(context, it) },
                maxPerRow = 2,
                stateOf = { TokenState.Available },
            )
        }

        SectionLabel(stringResource(R.string.capability_fps))
        if (camera.fps.isEmpty()) {
            EmptyState(stringResource(R.string.capability_none))
        } else {
            TokenRow(
                tokens = camera.fps.map { stringResource(R.string.format_fps, it) },
                maxPerRow = 3,
                stateOf = { TokenState.Available },
            )
        }

        LabeledValue(
            label = stringResource(R.string.capability_zoom),
            value = stringResource(R.string.zoom_factor, camera.maxZoom),
        )
        LabeledValue(
            label = stringResource(R.string.capability_orientation),
            value = stringResource(R.string.rotation_degrees, camera.orientationDegrees),
        )
    }
}
