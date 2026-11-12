package com.phonecam.camera.ui.components

import android.content.Context
import com.phonecam.camera.R
import com.phonecam.camera.camera.VideoPreset
import com.phonecam.camera.protocol.CameraInfo
import com.phonecam.camera.session.SessionState

/**
 * Presentation helpers shared by the cards.
 *
 * They live here rather than in the view model because they are pure formatting:
 * no I/O, no state, and the same function is used by the status card and the
 * notification so the two can never disagree about what "Warm" means.
 */

/** The session phase, in the user's words. */
fun phaseLabel(context: Context, phase: SessionState.Phase): String = context.getString(
    when (phase) {
        SessionState.Phase.IDLE -> R.string.phase_idle
        SessionState.Phase.DISCOVERING -> R.string.phase_discovering
        SessionState.Phase.CONNECTING -> R.string.phase_connecting
        SessionState.Phase.CONNECTED -> R.string.phase_connected
        SessionState.Phase.STREAMING -> R.string.phase_streaming
        SessionState.Phase.RECONNECTING -> R.string.phase_reconnecting
        SessionState.Phase.ERROR -> R.string.phase_error
    },
)

/** The thermal vocabulary from the wire, in prose. */
fun thermalLabel(context: Context, thermal: String): String = context.getString(
    when (thermal) {
        "light" -> R.string.thermal_light
        "moderate" -> R.string.thermal_moderate
        "severe" -> R.string.thermal_severe
        "critical" -> R.string.thermal_critical
        "emergency" -> R.string.thermal_emergency
        "shutdown" -> R.string.thermal_shutdown
        else -> R.string.thermal_none
    },
)

/**
 * The transport of a link that is up, or "Not connected" when there is none.
 *
 * [SessionState.transport] is the transport a dial is *using*, set the moment a
 * connection starts and reset to "none" on teardown; it is a fact about the
 * attempt, not about a link. The Connection row used to render it raw, so an
 * idle or disconnected phone claimed "Local network" (and the cold-start default
 * `"lan"` claimed it too), which is the same defect as a USB dial announcing a
 * cable before the PC answered. Callers pass [SessionState.connectedTransport],
 * which is null unless a link is really up.
 */
fun transportLabel(context: Context, transport: String?): String = context.getString(
    when (transport) {
        "usb" -> R.string.transport_usb
        "lan" -> R.string.transport_lan
        else -> R.string.transport_none
    },
)

fun qualityModeLabel(context: Context, mode: String): String = context.getString(
    if (mode == "manual") R.string.quality_manual else R.string.quality_auto,
)

fun screenModeLabel(context: Context, mode: String): String = context.getString(
    when (mode) {
        "dim" -> R.string.screen_dim
        "black" -> R.string.screen_black
        else -> R.string.screen_keep_on
    },
)

/** `"1920x1080"` → `"1920 x 1080"`; anything unexpected is passed through. */
fun resolutionLabel(context: Context, resolution: String?): String {
    if (resolution.isNullOrBlank()) return context.getString(R.string.value_unknown)
    val parsed = VideoPreset.parseResolution(resolution) ?: return resolution
    return context.getString(R.string.format_resolution, parsed.first, parsed.second)
}

fun bitrateLabel(context: Context, bitsPerSecond: Int): String {
    if (bitsPerSecond <= 0) return context.getString(R.string.value_unknown)
    return context.getString(R.string.format_bitrate, bitsPerSecond / 1_000_000.0)
}

fun fpsLabel(context: Context, fps: Double): String {
    if (fps <= 0.0) return context.getString(R.string.value_unknown)
    return context.getString(R.string.format_fps_decimal, fps)
}

fun percentLabel(context: Context, value: Double): String =
    context.getString(R.string.format_dropped, value)

/** A camera's human name, from the capability the desktop will also see. */
fun cameraLabel(context: Context, camera: CameraInfo?): String {
    if (camera == null) return context.getString(R.string.value_unknown)
    return when (camera.facing) {
        "back" -> context.getString(R.string.camera_back)
        "front" -> context.getString(R.string.camera_front)
        "external" -> context.getString(R.string.camera_external)
        // A real camera exists but its facing is one this build does not know;
        // naming it by id beats a generic "Camera" the user cannot act on.
        else -> context.getString(R.string.camera_id, camera.id)
    }
}

/**
 * The camera the session is on, or "Not streaming" when it has not looked yet.
 *
 * A cold session has no capabilities, so [SessionState.activeCamera] is null and
 * [cameraLabel] would answer "Unknown" — but nothing is being sent, which is a
 * fact rather than a gap.
 */
fun activeCameraLabel(context: Context, state: SessionState): String {
    val camera = state.activeCamera
    if (camera == null && !state.isStreaming) return context.getString(R.string.value_not_streaming)
    return cameraLabel(context, camera)
}

/**
 * The resolution actually being sent.
 *
 * Before the first encoder is configured there is nothing to report, and saying
 * so is more honest than "Unknown" (PRD §38: explain, do not guess).
 */
fun actualResolutionLabel(context: Context, state: SessionState): String {
    if (state.actualResolution.isNullOrBlank() && !state.isStreaming) {
        return context.getString(R.string.value_not_streaming)
    }
    return resolutionLabel(context, state.actualResolution)
}

/** The frame rate actually being sent, in the same voice as [actualResolutionLabel]. */
fun actualFpsLabel(context: Context, state: SessionState): String {
    val fps = state.actualFps
    if (fps == null) {
        return context.getString(
            if (state.isStreaming) R.string.value_unknown else R.string.value_not_streaming,
        )
    }
    return context.getString(R.string.format_fps, fps)
}

/**
 * Why a camera cannot be selected from this phone, or `null` when it can.
 *
 * The picker disables a camera the probe reported but that has nothing to stream
 * with, rather than offering a switch that would fail after the fact.
 */
fun cameraUnavailableReason(context: Context, camera: CameraInfo): String? {
    if (camera.resolutions.isEmpty()) return context.getString(R.string.reason_camera_no_resolutions)
    return null
}

