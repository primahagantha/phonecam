package com.phonecam.camera.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.warning

/**
 * The phase indicator: one pill, one word, one dot.
 *
 * The seven session phases collapse onto four visual classes, because what the
 * user needs from this pill is not "which of seven states am I in" but "is
 * anything wrong, and do I need to do something".
 *
 *  - [PhaseClass.Live] and [PhaseClass.Ready] share the accent. Both are healthy
 *    and neither wants an action; the word beside the dot says which one it is.
 *    Giving them different colours would imply a difference in urgency that does
 *    not exist.
 *  - [PhaseClass.Working] is the warning hue, because a connection that is still
 *    being established is the one case where waiting is the correct response.
 *  - [PhaseClass.Problem] is the error hue and always comes with prose in the
 *    error card above.
 *  - [PhaseClass.Idle] is neutral. Nothing is running, and that is not a problem.
 *
 * Colour is never the only signal: the phase is written out beside the dot, so
 * the pill still works for a user who cannot distinguish the hues, and in bright
 * sunlight where green and amber converge.
 */
@Composable
fun PhasePill(state: SessionState, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val label = phaseLabel(context, state.phase)
    val accent = phaseAccent(state.phase)
    // Read outside the semantics lambda: `stringResource` is composable and the
    // lambda is not.
    val classLabel = phaseClassLabel(context, state.phase)

    Row(
        modifier = modifier
            .clip(PhoneCamShapes.Control)
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .border(1.dp, MaterialTheme.colorScheme.outline, PhoneCamShapes.Control)
            .padding(horizontal = 12.dp, vertical = 6.dp)
            // The visible text already reads the phase; this adds the class
            // ("problem") so a screen reader user gets the same glance the dot
            // gives everyone else.
            .semantics { stateDescription = classLabel },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        // The dot is a redundant encoding, not decoration: it carries the phase
        // class, and the text beside it carries the phase. Each one alone is
        // enough to use the screen, which is the point.
        Row(
            modifier = Modifier
                .size(10.dp)
                .clip(CircleShape)
                .background(accent),
        ) {}
        Text(
            text = label,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurface,
        )
    }
}

/** The four things the pill can say. */
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

/**
 * Every dot colour clears 3:1 against the pill's `surfaceVariant` background,
 * which is the WCAG minimum for a non-text indicator.
 */
@Composable
private fun phaseAccent(phase: SessionState.Phase): Color = when (phaseClass(phase)) {
    PhaseClass.Live, PhaseClass.Ready -> MaterialTheme.colorScheme.primary
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
