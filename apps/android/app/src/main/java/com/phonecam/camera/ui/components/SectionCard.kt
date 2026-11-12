package com.phonecam.camera.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.ui.theme.PhoneCamShapes

/**
 * The shapes this screen is built from.
 *
 * Three containers, and the rule for picking one:
 *
 *  - [SectionCard] is a panel. Used only where the grouping itself is the point:
 *    the things a user comes here to check or change. Panels are counted, not
 *    sprinkled: a screen of identical panels reads as a form to fill in rather
 *    than a state to glance at.
 *  - [PlainSection] is everything else. Same title, no background, opened by a
 *    hairline. Reference material lives here.
 *  - [InsetWell] is for content that is itself a surface, such as the log.
 *
 * The panels are flat: one surface tone plus a hairline, no shadow. On a phone
 * held at arm's length in a dark room a shadow reads as grime while a hairline
 * reads as an edge, and the two surface tones are only 1.16:1 apart, so the edge
 * is carrying the separation either way.
 */

/** A panel: a titled block of the screen, with an edge and a background. */
@Composable
fun SectionCard(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    action: (@Composable () -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(PhoneCamShapes.Panel)
            .background(MaterialTheme.colorScheme.surface)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, PhoneCamShapes.Panel)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        SectionHeading(title = title, action = action)
        if (subtitle != null) {
            Text(
                text = subtitle,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        content()
    }
}

/**
 * A titled block with no panel around it, opened by a hairline.
 *
 * For content the user reads once rather than acts on, which does not deserve the
 * visual weight of a card.
 */
@Composable
fun PlainSection(
    title: String,
    modifier: Modifier = Modifier,
    action: (@Composable () -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Hairline()
        SectionHeading(title = title, action = action)
        content()
    }
}

/** The title row shared by both container styles. */
@Composable
private fun SectionHeading(title: String, action: (@Composable () -> Unit)?) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.weight(1f),
        )
        action?.invoke()
    }
}

/** A sub-heading inside a container, for when one block holds two ideas. */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier,
    )
}

/** A 1dp separator. Structural only: it groups, it never means anything. */
@Composable
fun Hairline(modifier: Modifier = Modifier) {
    Spacer(
        modifier = modifier
            .fillMaxWidth()
            .height(1.dp)
            .background(MaterialTheme.colorScheme.outlineVariant),
    )
}

/**
 * A label and its value.
 *
 * Both halves get a guaranteed share of the row so a long value (a camera name, a
 * full address) wraps inside its own column instead of squeezing the label down
 * to an ellipsis. The value is end-aligned so a column of rows scans as a column
 * of values.
 */
@Composable
fun LabeledValue(
    label: String,
    value: String,
    modifier: Modifier = Modifier,
    valueColor: Color = MaterialTheme.colorScheme.onSurface,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.weight(1f),
        )
        Text(
            text = value,
            style = MaterialTheme.typography.bodyMedium,
            color = valueColor,
            textAlign = TextAlign.End,
            modifier = Modifier.weight(1.5f),
        )
    }
}

/**
 * An empty state: one sentence saying what is missing and how to fix it.
 *
 * The copy always carries the way out, because "nothing here" with no next step
 * is the state a user gets stuck in. It is prose on the background rather than a
 * boxed message so an empty list does not compete with the content that will
 * replace it.
 */
@Composable
fun EmptyState(text: String, modifier: Modifier = Modifier) = MutedProse(text, modifier)

/**
 * Explanatory prose under a heading: what a block means, or how to read it.
 *
 * Rendered identically to [EmptyState] and kept as a separate name because the
 * two carry different obligations. An empty state must say how to fill itself; a
 * hint must say something the heading alone does not.
 */
@Composable
fun Hint(text: String, modifier: Modifier = Modifier) = MutedProse(text, modifier)

@Composable
private fun MutedProse(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.fillMaxWidth(),
    )
}

/** A recessed well for content that is itself a surface, such as the log. */
@Composable
fun InsetWell(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(PhoneCamShapes.Field)
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
        content = content,
    )
}

/**
 * What a token is telling the user.
 *
 * The states are distinct because a token is not a boolean: "this camera has a
 * torch" and "the torch is on" are different facts, and a disabled switch can
 * only express the second one badly. These tokens express the first one, which is
 * what a capability list is for. [Neutral] exists for a value that carries no
 * claim at all — a discovered PC whose beacon established nothing beyond its
 * presence — so the token does not colour "we know nothing more" as "available".
 */
enum class TokenState {
    /** The stream is using this right now. */
    InUse,

    /** This phone can do it, and the PC is not asking for it. */
    Available,

    /** This phone cannot do it. */
    Unavailable,

    /** No claim: the value is stated without a status either way. */
    Neutral,
}

/**
 * One value read as part of a set: a quality rung, a resolution, a capability.
 *
 * Fill and text weight carry the state, so the token row is readable without a
 * legend. [TokenState.Unavailable] is deliberately not struck through or greyed
 * to invisibility: it still has to be legible, because "this camera has no flash"
 * is information the user came for.
 *
 * The state is also announced, because it is carried by colour and fill alone and
 * a screen reader user would otherwise hear a bare list of resolutions with no
 * idea which one is live.
 */
@Composable
fun StatusToken(
    text: String,
    state: TokenState,
    modifier: Modifier = Modifier,
) {
    val container = when (state) {
        TokenState.InUse -> MaterialTheme.colorScheme.primaryContainer
        TokenState.Available -> MaterialTheme.colorScheme.surfaceVariant
        TokenState.Unavailable -> Color.Transparent
        TokenState.Neutral -> Color.Transparent
    }
    val content = when (state) {
        TokenState.InUse -> MaterialTheme.colorScheme.onPrimaryContainer
        TokenState.Available -> MaterialTheme.colorScheme.onSurface
        TokenState.Unavailable -> MaterialTheme.colorScheme.onSurfaceVariant
        TokenState.Neutral -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    val outline = when (state) {
        TokenState.InUse -> MaterialTheme.colorScheme.primaryContainer
        TokenState.Available -> MaterialTheme.colorScheme.surfaceVariant
        TokenState.Unavailable -> MaterialTheme.colorScheme.outlineVariant
        TokenState.Neutral -> MaterialTheme.colorScheme.outlineVariant
    }
    val stateLabel = stringResource(
        when (state) {
            TokenState.InUse -> R.string.token_state_in_use
            TokenState.Available -> R.string.token_state_available
            TokenState.Unavailable -> R.string.token_state_unavailable
            TokenState.Neutral -> R.string.token_state_none
        },
    )
    Text(
        text = text,
        style = MaterialTheme.typography.bodySmall,
        color = content,
        modifier = modifier
            .clip(PhoneCamShapes.Control)
            .background(container)
            .border(1.dp, outline, PhoneCamShapes.Control)
            .padding(horizontal = 10.dp, vertical = 5.dp)
            .semantics { stateDescription = stateLabel },
    )
}

/**
 * A wrapping row of [StatusToken]s.
 *
 * Hand-rolled rather than `FlowRow` because the token sets here are small and
 * fixed (four quality rungs, a handful of resolutions), and chunking them into
 * rows explicitly keeps the vertical rhythm identical to every other block on the
 * screen instead of depending on a layout's internal line height.
 */
@Composable
fun TokenRow(
    tokens: List<String>,
    stateOf: (String) -> TokenState,
    modifier: Modifier = Modifier,
    maxPerRow: Int = 3,
) {
    if (tokens.isEmpty()) return
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        tokens.chunked(maxPerRow).forEach { row ->
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                row.forEach { token ->
                    StatusToken(text = token, state = stateOf(token))
                }
            }
        }
    }
}
