package com.phonecam.camera.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.ui.theme.PhoneCamShapes

/**
 * A studio panel: crisp obsidian surface (`#10151C`), `1dp` border (`#1D2633`),
 * generous `16dp` padding and `12dp` internal rhythm.
 */
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
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            SectionHeading(title = title, action = action)
            if (subtitle != null) {
                Text(
                    text = subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        content()
    }
}

/**
 * A titled block with no card frame, opened by a hairline for reference material.
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

/** A sub-heading inside a container. */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier,
    )
}

/** A 1dp horizontal structural hairline. */
@Composable
fun Hairline(modifier: Modifier = Modifier) {
    Spacer(
        modifier = modifier
            .fillMaxWidth()
            .height(1.dp)
            .background(MaterialTheme.colorScheme.outlineVariant),
    )
}

/** A 1dp vertical structural hairline (used beside the landscape NavigationRail). */
@Composable
fun VerticalHairline(modifier: Modifier = Modifier) {
    Spacer(
        modifier = modifier
            .fillMaxHeight()
            .width(1.dp)
            .background(MaterialTheme.colorScheme.outlineVariant),
    )
}

/**
 * A key-value row with tabular numerals (`tnum`) on values so IP addresses,
 * frame rates, bitrates, and percentages align cleanly without jitter.
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
            style = MaterialTheme.typography.bodyMedium.copy(
                fontFeatureSettings = "tnum",
            ),
            color = valueColor,
            textAlign = TextAlign.End,
            modifier = Modifier.weight(1.45f),
        )
    }
}

/** An honest empty state naming what is missing and the next action. */
@Composable
fun EmptyState(text: String, modifier: Modifier = Modifier) = MutedProse(text, modifier)

/** Explanatory prose under a heading. */
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

/** A recessed obsidian well (`#18212C`) for logs, release notes, or grouped list rows. */
@Composable
fun InsetWell(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(PhoneCamShapes.Field)
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, PhoneCamShapes.Field)
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
        content = content,
    )
}

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
 * Compact status/capability token using crisp 8dp studio chip geometry.
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
        TokenState.InUse -> MaterialTheme.colorScheme.primary.copy(alpha = 0.55f)
        TokenState.Available -> MaterialTheme.colorScheme.outline
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
        style = MaterialTheme.typography.bodySmall.copy(fontFeatureSettings = "tnum"),
        color = content,
        modifier = modifier
            .clip(PhoneCamShapes.Chip)
            .background(container)
            .border(1.dp, outline, PhoneCamShapes.Chip)
            .padding(horizontal = 10.dp, vertical = 5.dp)
            .semantics { stateDescription = stateLabel },
    )
}

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
