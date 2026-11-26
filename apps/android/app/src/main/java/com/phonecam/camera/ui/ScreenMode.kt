package com.phonecam.camera.ui

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.view.WindowManager
import android.view.WindowManager.LayoutParams
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.platform.LocalContext

/**
 * The screen behaviours the desktop can ask for (`set_screen`, protocol.md §6.2).
 *
 * These exist because the phone is pointed at the subject, not at the user, so
 * what the panel *does* matters more than what it shows. `DIM` and `BLACK` cut
 * the light coming off the phone — which otherwise lands on the subject as a
 * colour cast — without stopping capture.
 *
 * They are implemented as window brightness overrides rather than by letting the
 * screen sleep, because a sleeping screen stops the preview and, on some devices,
 * the capture session itself. The camera device must never disappear (PRD §16),
 * and that includes while the screen is dark.
 */
enum class ScreenMode(val wire: String, val brightness: Float) {
    /** Normal brightness, screen held awake. */
    KEEP_ON("keepOn", WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE),

    /** Awake but dark, for a phone left face-up next to the subject. */
    DIM("dim", 0.05f),

    /**
     * Awake at minimum brightness with a black surface over the UI. The panel is
     * never actually off, so capture continues; the black surface is what makes
     * it read as off in a dark room, where minimum brightness would still glow.
     */
    BLACK("black", 0.0f);

    /**
     * Whether the screen is pinned on in this mode.
     *
     * Kept `true` across all streaming screen modes (`KEEP_ON`, `DIM`, `BLACK`)
     * so the platform never turns off or locks the display while streaming,
     * which would interrupt capture or hide the OLED blackout ambient display.
     */
    val keepScreenOn: Boolean
        get() = true

    companion object {
        /** Unknown values fall back to [KEEP_ON], the pre-existing behaviour. */
        fun fromWire(value: String?): ScreenMode =
            entries.firstOrNull { it.wire == value } ?: KEEP_ON
    }
}

/**
 * Applies [mode] to the host window while [active] is true.
 *
 * Scoped to streaming: a mode only means something while the phone is acting as
 * a camera, and holding the screen awake outside that would drain the battery of
 * a phone someone merely opened the app on.
 *
 * The override belongs to the Activity window, so it is released when the app
 * leaves the foreground. That is correct — the stream is owned by the foreground
 * service and keeps running; only the screen returns to normal.
 */
@Composable
fun ApplyScreenMode(mode: ScreenMode, active: Boolean) {
    val context = LocalContext.current
    val activity = context.findActivity()

    DisposableEffect(mode, active, activity) {
        val window = activity?.window
        if (window == null || !active) return@DisposableEffect onDispose { }

        val previousBrightness = window.attributes.screenBrightness
        // Keep FLAG_KEEP_SCREEN_ON set across KEEP_ON, DIM, and BLACK while streaming
        // so the display does not sleep/lock and kill capture or the ambient display.
        if (mode.keepScreenOn) {
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
        window.attributes = window.attributes.apply { screenBrightness = mode.brightness }

        onDispose {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            window.attributes = window.attributes.apply { screenBrightness = previousBrightness }
        }
    }
}

/** Compose hands back the base context, which may be a wrapper around the Activity. */
private fun Context.findActivity(): Activity? {
    var current: Context? = this
    while (current is ContextWrapper) {
        if (current is Activity) return current
        current = current.baseContext
    }
    return null
}
