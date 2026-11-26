package com.phonecam.camera.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.PhoneCamStudioTheme
import com.phonecam.camera.ui.theme.warning

/**
 * The phase indicator: one pill, one semantic dot, one word.
 *
 * Colour carries real session state and is always paired with text:
 *  - [PhaseClass.Live] / [PhaseClass.Ready]: semantic Live Green (`#22C55E`).
 *  - [PhaseClass.Working]: Warning Amber (`#F59E0B`).
 *  - [PhaseClass.Problem]: Danger Red (`#EF4444`).
 *  - [PhaseClass.Idle]: muted slate neutral.
 */
@Composable
fun PhasePill(
    state: SessionState,
    modifier: Modifier = Modifier,
    compact: Boolean = false,
) {
    val context = LocalContext.current
    val studio = PhoneCamStudioTheme.colors
    val label = phaseLabel(context, state.phase)
    val kind = phaseClass(state.phase)
    val accent = phaseAccent(kind)
    val classLabel = phaseClassLabel(context, state.phase)

    val containerColor = when (kind) {
        PhaseClass.Live -> studio.liveContainer
        PhaseClass.Problem -> studio.dangerSurface
        else -> MaterialTheme.colorScheme.surfaceVariant
    }
    val borderColor = when (kind) {
        PhaseClass.Live -> studio.live.copy(alpha = 0.45f)
        PhaseClass.Problem -> studio.dangerBorder
        else -> MaterialTheme.colorScheme.outline
    }
    val textColor = when (kind) {
        PhaseClass.Live -> studio.onLiveContainer
        PhaseClass.Problem -> studio.dangerText
        else -> MaterialTheme.colorScheme.onSurface
    }

    Row(
        modifier = modifier
            .clip(PhoneCamShapes.Control)
            .background(containerColor)
            .border(1.dp, borderColor, PhoneCamShapes.Control)
            .padding(
                horizontal = if (compact) 10.dp else 12.dp,
                vertical = if (compact) 4.dp else 5.dp,
            )
            .semantics { stateDescription = classLabel },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(
            modifier = Modifier
                .size(if (compact) 7.dp else 8.dp)
                .clip(CircleShape)
                .background(accent),
        )
        Text(
            text = label,
            style = if (compact) MaterialTheme.typography.labelMedium else MaterialTheme.typography.labelLarge,
            color = textColor,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

private enum class PhaseClass { Live, Ready, Working, Problem, Idle }

private fun phaseClass(phase: SessionState.Phase): PhaseClass = when (phase) {
    SessionState.Phase.STREAMING -> PhaseClass.Live
    SessionState.Phase.CONNECTED -> PhaseClass.Ready
    SessionState.Phase.DISCOVERING,
    SessionState.Phase.CONNECTING,
    SessionState.Phase.RECONNECTING,
    -> PhaseClass.Working

    SessionState.Phase.ERROR -> PhaseClass.Problem
    SessionState.Phase.IDLE -> PhaseClass.Idle
}

@Composable
private fun phaseAccent(kind: PhaseClass): Color = when (kind) {
    PhaseClass.Live, PhaseClass.Ready -> PhoneCamStudioTheme.colors.live
    PhaseClass.Working -> MaterialTheme.colorScheme.warning
    PhaseClass.Problem -> MaterialTheme.colorScheme.error
    PhaseClass.Idle -> MaterialTheme.colorScheme.onSurfaceVariant
}

@Composable
private fun phaseClassLabel(context: android.content.Context, phase: SessionState.Phase): String =
    context.getString(
        when (phaseClass(phase)) {
            PhaseClass.Live -> R.string.phase_class_live
            PhaseClass.Ready -> R.string.phase_class_ready
            PhaseClass.Working -> R.string.phase_class_working
            PhaseClass.Problem -> R.string.phase_class_problem
            PhaseClass.Idle -> R.string.phase_class_idle
        },
    )
