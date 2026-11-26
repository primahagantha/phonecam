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
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.PhoneCamStudioTheme

/**
 * Camera switcher card gated on hardware [CameraInfo].
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
 * Output settings read-only summary driven by PhoneCam Desktop.
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

@Composable
private fun OutputSettings(state: SessionState) {
    val context = LocalContext.current
    val studio = PhoneCamStudioTheme.colors

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
    LabeledValue(
        label = stringResource(R.string.control_anti_flicker),
        value = stringResource(
            when (state.antiFlicker.lowercase()) {
                "50hz" -> R.string.anti_flicker_50hz
                "60hz" -> R.string.anti_flicker_60hz
                "off" -> R.string.anti_flicker_off
                else -> R.string.anti_flicker_auto
            },
        ),
    )
    if (state.appliedTorch()) {
        LabeledValue(
            label = stringResource(R.string.control_torch),
            value = stringResource(R.string.value_on),
            valueColor = studio.live,
        )
    }
}

@Composable
private fun CameraPicker(
    cameras: List<CameraInfo>,
    selectedId: String?,
    onSelect: (String) -> Unit,
) {
    val context = LocalContext.current
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
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
        cameras.forEach { camera ->
            cameraUnavailableReason(context, camera)?.let { reason ->
                EmptyState(text = "${cameraLabel(context, camera)}: $reason")
            }
        }
    }
}

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
        MaterialTheme.colorScheme.primaryContainer
    } else {
        MaterialTheme.colorScheme.surfaceVariant
    }
    val content = if (selected) {
        MaterialTheme.colorScheme.onPrimaryContainer
    } else {
        MaterialTheme.colorScheme.onSurface
    }
    val borderColor = if (selected) {
        MaterialTheme.colorScheme.primary
    } else {
        MaterialTheme.colorScheme.outline
    }

    Surface(
        onClick = onClick,
        enabled = enabled,
        shape = PhoneCamShapes.Panel,
        color = container,
        contentColor = content,
        border = BorderStroke(if (selected) 1.5.dp else 1.dp, borderColor),
        tonalElevation = 0.dp,
        modifier = Modifier
            .widthIn(min = 128.dp)
            .semantics { contentDescription = description },
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Icon(imageVector = icon, contentDescription = null, modifier = Modifier.size(34.dp))
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

private fun cameraIcon(camera: CameraInfo) = when (camera.facing) {
    "front" -> Icons.Filled.CameraFront
    "back" -> Icons.Filled.CameraRear
    "external" -> Icons.Filled.Videocam
    else -> Icons.Filled.PhotoCamera
}

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
    val studio = PhoneCamStudioTheme.colors
    val tint = if (supported) {
        studio.live
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
                modifier = Modifier.size(16.dp),
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

@Composable
fun CameraCapabilitySection(camera: CameraInfo?, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    SectionCard(
        title = stringResource(R.string.camera_capabilities_title),
        subtitle = if (camera != null) stringResource(R.string.camera_capabilities_note) else null,
        modifier = modifier,
    ) {
        if (camera == null) {
            EmptyState(stringResource(R.string.capability_none))
            return@SectionCard
        }

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

        Hairline()
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
