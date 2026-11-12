package com.phonecam.camera.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.phonecam.camera.R
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.session.connectedTransport
import com.phonecam.camera.session.hasLiveHealth
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.onWarningContainer
import com.phonecam.camera.ui.theme.warning
import com.phonecam.camera.ui.theme.warningContainer

/**
 * What is happening right now, and what the phone is really sending.
 *
 * "Requested" and "Sending" are deliberately separate rows. A stream that was
 * asked for 1080p60 and is sending 720p30 is working as designed (PRD §13), and
 * showing both numbers is how the user sees that instead of suspecting a fault.
 *
 * It lives on the Output screen because that is what it describes: the shape of
 * the stream the desktop receives. The camera's own facts are on the Camera
 * screen, so the same number is never on screen twice.
 */
@Composable
fun StatusCard(state: SessionState, modifier: Modifier = Modifier) {
    val context = LocalContext.current

    SectionCard(title = stringResource(R.string.section_status), modifier = modifier) {
        PhasePill(state)

        state.serverName?.let { name ->
            LabeledValue(label = stringResource(R.string.label_pc), value = name)
        }
        LabeledValue(
            label = stringResource(R.string.label_transport),
            value = transportLabel(context, state.connectedTransport()),
        )
        if (state.host != null && state.port != null) {
            LabeledValue(
                label = stringResource(R.string.label_address),
                value = "${state.host}:${state.port}",
            )
        }

        // The negotiation pair, and the reason this card exists. A stream asked
        // for 1080p60 that is sending 720p30 is working as designed (PRD §13),
        // and showing both numbers is how the user sees that instead of
        // suspecting a fault. "Sending" collapses to one phrase rather than two
        // when nothing is streaming, because "Not streaming · Not streaming" is
        // noise.
        LabeledValue(
            label = stringResource(R.string.label_requested_quality),
            value = requestedQuality(context, state),
        )
        LabeledValue(
            label = stringResource(R.string.label_actual_quality),
            value = actualQuality(context, state),
        )

        // The throughput block only exists while there is throughput. Before
        // that, "Dropped frames: 0.0%" is a row of zeroes pretending to be a
        // measurement.
        if (state.isStreaming) {
            Hairline()
            LabeledValue(
                label = stringResource(R.string.label_stream_fps),
                value = fpsLabel(context, state.streamFps),
            )
            LabeledValue(
                label = stringResource(R.string.label_bitrate),
                value = bitrateLabel(context, state.bitrate),
            )
            LabeledValue(
                label = stringResource(R.string.label_dropped),
                value = percentLabel(context, state.droppedPercent),
            )
        }

        Hairline()
        LabeledValue(
            label = stringResource(R.string.label_battery),
            value = batteryValue(context, state),
        )
        LabeledValue(
            label = stringResource(R.string.label_thermal),
            value = thermalValue(context, state),
            valueColor = thermalColor(state),
        )
    }
}

/**
 * What the desktop asked for. "Auto" is a real answer here, not a missing one:
 * the desktop may deliberately let the phone choose.
 */
@Composable
private fun requestedQuality(context: android.content.Context, state: SessionState): String {
    val resolution = if (state.requestedResolution == "auto") {
        stringResource(R.string.value_auto)
    } else {
        resolutionLabel(context, state.requestedResolution)
    }
    val fps = state.requestedFps
        ?.let { stringResource(R.string.format_fps, it) }
        ?: stringResource(R.string.value_auto)
    return "$resolution · $fps"
}

/** What is actually going out. One phrase, not two, when nothing is streaming. */
private fun actualQuality(context: android.content.Context, state: SessionState): String {
    if (state.actualResolution.isNullOrBlank() && !state.isStreaming) {
        return context.getString(R.string.value_not_streaming)
    }
    return "${actualResolutionLabel(context, state)} · ${actualFpsLabel(context, state)}"
}

@Composable
private fun batteryValue(context: android.content.Context, state: SessionState): String {
    if (state.batteryPercent < 0) return stringResource(R.string.value_unknown)
    val percent = stringResource(R.string.format_percent, state.batteryPercent)
    val charging = stringResource(
        if (state.charging) R.string.battery_charging else R.string.battery_on_battery,
    )
    return "$percent · $charging"
}

/**
 * The phone's temperature, or "Unknown" when no reading has been taken.
 *
 * `thermal` defaults to `"none"` and is assigned only by the once-per-second stats
 * tick, which runs only while streaming, so outside a stream the raw field is not
 * a measurement. Rendering it read "Normal" for a phone nobody had checked — the
 * same defect the sibling battery row already guards with its `-1` sentinel, and
 * the reason `"none"` cannot be mapped to "Unknown" in the label: a genuine
 * "none" is a real, normal reading. The gate is [SessionState.hasLiveHealth], the
 * same one the health advice below uses, so the row and the advice cannot
 * disagree.
 *
 * The unknown case reuses [R.string.value_unknown] rather than a thermal-specific
 * string because it is the same situation as the battery row directly above it —
 * a reading the phone has not taken — and one word for one situation is more
 * honest than a second word implying a different kind of gap.
 */
@Composable
private fun thermalValue(context: android.content.Context, state: SessionState): String {
    if (!state.hasLiveHealth) return stringResource(R.string.value_unknown)
    return thermalLabel(context, state.thermal)
}

/**
 * Temperature is the one reading that is not neutral when it is high: at "severe"
 * and above the stream is about to degrade, so the value takes the warning hue.
 * The words carry the same fact, so the colour is a second encoding, not the only
 * one.
 *
 * An unmeasured reading gets no health colour: colouring "Unknown" would imply a
 * reading was taken, which is the claim this row exists to avoid.
 */
@Composable
private fun thermalColor(state: SessionState) = when {
    !state.hasLiveHealth -> MaterialTheme.colorScheme.onSurface
    state.thermal == "severe" || state.thermal == "critical" ||
        state.thermal == "emergency" || state.thermal == "shutdown" ->
        MaterialTheme.colorScheme.warning

    else -> MaterialTheme.colorScheme.onSurface
}

/**
 * The thermal and battery warnings, shown only when there is something to say.
 *
 * These are advice, not failures, so they get the warning container rather than
 * the error one: nothing is broken, the user just has a choice to make. When
 * there is no advice this renders nothing at all, which is why it is allowed to
 * sit above the cards without reserving space for itself.
 */
@Composable
fun HealthCard(state: SessionState, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val advice = healthAdviceText(context, state) ?: return

    AdvisoryBanner(
        title = stringResource(R.string.banner_local_title),
        text = advice,
        modifier = modifier,
    )
}

/**
 * The thermal and battery thresholds, using the same numbers the session uses.
 *
 * The copy is duplicated from [com.phonecam.camera.protocol.DeviceHealth] as
 * resources rather than called from it, because that class returns English
 * strings for the log and for the desktop, while everything the *screen* shows
 * has to be translatable (PRD §38).
 *
 * The advice is present-tense ("Your phone is warming up"), and the reading it
 * comes from is sampled by the once-per-second stats tick, which runs only while
 * a stream does. So the advice is withheld once the stream stops: the last
 * sample is then a fact about a stream that is over, and repeating it would keep
 * warning about a phone that is already cool and idle. See
 * [SessionState.hasLiveHealth].
 */
private fun healthAdviceText(context: android.content.Context, state: SessionState): String? {
    if (!state.hasLiveHealth) return null
    return when {
        state.thermal == "severe" || state.thermal == "critical" ||
            state.thermal == "emergency" || state.thermal == "shutdown" ->
            context.getString(R.string.advice_thermal_severe)

        state.thermal == "moderate" -> context.getString(R.string.advice_thermal_moderate)

        state.charging -> null

        state.batteryPercent in 0..10 -> context.getString(R.string.advice_battery_very_low)

        state.batteryPercent in 11..20 -> context.getString(R.string.advice_battery_low)

        else -> null
    }
}

/**
 * An inline advisory: a condition worth knowing about, with no action required.
 *
 * Distinct from [ErrorCard] (something failed, here is the way out) and from
 * [InfoBanner] (a request was substituted, dismiss this). An advisory is about
 * the phone itself and the user cannot dismiss it, because the condition has not
 * gone away.
 */
@Composable
fun AdvisoryBanner(
    title: String,
    text: String,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(PhoneCamShapes.Panel)
            .background(MaterialTheme.colorScheme.warningContainer)
            .border(1.dp, MaterialTheme.colorScheme.warning, PhoneCamShapes.Panel)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onWarningContainer,
        )
        Text(
            text = text,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onWarningContainer,
        )
    }
}
