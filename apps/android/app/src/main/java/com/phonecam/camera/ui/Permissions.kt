package com.phonecam.camera.ui

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.core.content.ContextCompat

/**
 * Runtime permissions, in one place.
 *
 * They live here rather than inside `MainActivity` because two screens need the
 * camera: the main screen streams with it, and the QR scanner reads a pairing
 * code with it. When only the main screen knew how to ask, opening the scanner on
 * a phone that had never been asked produced a black viewfinder with no prompt
 * and no explanation, which is exactly what the user saw.
 */
internal fun Context.hasCameraPermission(): Boolean =
    ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) ==
        PackageManager.PERMISSION_GRANTED

internal fun Context.hasNotificationPermission(): Boolean {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return true
    return ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) ==
        PackageManager.PERMISSION_GRANTED
}

/**
 * What to ask for when the user wants the camera.
 *
 * Notifications are bundled in because both prompts are about the same
 * intention — "I want to stream" — and asking twice for two unrelated switches
 * is the kind of thing that makes a permission dialog feel like nagging.
 */
internal fun Context.permissionsToRequest(): Array<String> = buildList {
    add(Manifest.permission.CAMERA)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        add(Manifest.permission.POST_NOTIFICATIONS)
    }
}.toTypedArray()

/**
 * Whether the platform will still show a camera dialog.
 *
 * After a permanent denial it will not, so the only honest affordance left is a
 * route to Settings (PRD §39: explain, do not nag).
 */
internal fun Context.shouldShowCameraRationale(): Boolean {
    val activity = this as? ComponentActivity ?: return false
    return activity.shouldShowRequestPermissionRationale(Manifest.permission.CAMERA)
}

/** Opens this app's page in system Settings, where the camera switch lives. */
internal fun Context.openAppSettings() {
    startActivity(
        Intent(
            Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
            Uri.fromParts("package", packageName, null),
        ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
    )
}

/** True when this context can host a permission request. */
internal fun Context.canRequestPermissions(): Boolean = this is Activity
