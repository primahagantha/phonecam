package com.phonecam.camera.data

import android.content.Context
import android.os.Build
import com.phonecam.camera.protocol.Auth
import com.phonecam.camera.protocol.hexToBytesOrNull

/**
 * Locally persisted device identity and paired-desktop records (PRD §36, §37).
 *
 * Storage is app-private `SharedPreferences`. The device secret never leaves the
 * phone except inside an HMAC, and is never logged (PRD §63).
 */
class IdentityStore(context: Context) {

    private val prefs = context.applicationContext
        .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /**
     * Stable id for this phone.
     *
     * Once a desktop has paired with this phone, the id it minted wins: the
     * desktop stores that id as the key to the device secret, so a phone that
     * re-authenticates with its own locally generated id is a device the desktop
     * has never heard of and is refused (docs/protocol.md §4.2 step 4). Before any
     * pairing there is nothing to defer to, so the local id is used and the
     * desktop adopts it during the approval flow.
     */
    val deviceId: String
        get() = prefs.getString(KEY_PAIRED_DEVICE_ID, null)
            ?: prefs.getString(KEY_DEVICE_ID, null) ?: generateDeviceId().also {
                prefs.edit().putString(KEY_DEVICE_ID, it).apply()
            }

    val deviceName: String
        get() = prefs.getString(KEY_DEVICE_NAME, null)
            ?: "${Build.MANUFACTURER} ${Build.MODEL}".trim().also {
                prefs.edit().putString(KEY_DEVICE_NAME, it).apply()
            }

    /** 32 random bytes, hex encoded. Null until a successful pairing. */
    var deviceSecretHex: String?
        get() = prefs.getString(KEY_DEVICE_SECRET, null)
        set(value) {
            prefs.edit().apply {
                if (value == null) remove(KEY_DEVICE_SECRET) else putString(KEY_DEVICE_SECRET, value)
            }.apply()
        }

    /** The desktop this phone last paired with, used for auto-connect (§29). */
    var pairedServerId: String?
        get() = prefs.getString(KEY_SERVER_ID, null)
        set(value) = prefs.edit().putString(KEY_SERVER_ID, value).apply()

    var pairedServerName: String?
        get() = prefs.getString(KEY_SERVER_NAME, null)
        set(value) = prefs.edit().putString(KEY_SERVER_NAME, value).apply()

    /** Last known reachable address, so a manual reconnect can skip discovery. */
    var lastHost: String?
        get() = prefs.getString(KEY_LAST_HOST, null)
        set(value) = prefs.edit().putString(KEY_LAST_HOST, value).apply()

    var lastPort: Int
        get() = prefs.getInt(KEY_LAST_PORT, 0)
        set(value) = prefs.edit().putInt(KEY_LAST_PORT, value).apply()

    /**
     * Whether the phone shows its own viewfinder while streaming.
     *
     * The viewfinder is a second output of the capture session, so it costs a
     * surface and a little power and heat. A phone mounted facing away is never
     * looked at, and turning the preview off is the difference between a warm
     * phone and a cool one over a long stream. Default on: a webcam with no
     * picture is hard to trust.
     */
    var previewEnabled: Boolean
        get() = prefs.getBoolean(KEY_PREVIEW_ENABLED, true)
        set(value) = prefs.edit().putBoolean(KEY_PREVIEW_ENABLED, value).apply()

    /**
     * Monotonic client nonce counter. Persisted so a restart cannot replay a
     * previously accepted nonce, which the desktop rejects (docs §4.4).
     */
    fun nextClientNonce(): Long {
        val previous = prefs.getLong(KEY_NONCE, 0L)
        val next = Auth.nextClientNonce(previous)
        prefs.edit().putLong(KEY_NONCE, next).apply()
        return next
    }

    /**
     * Records a completed pairing.
     *
     * [deviceId] is the id the desktop minted and stored as the key to this
     * phone's secret. Keeping it is what makes the *next* connection authenticate
     * as a known device instead of a stranger (docs §4.2 step 4, §4.4).
     */
    fun rememberPairing(
        serverId: String,
        serverName: String,
        secretHex: String,
        deviceId: String? = null,
    ) {
        prefs.edit().apply {
            putString(KEY_SERVER_ID, serverId)
            putString(KEY_SERVER_NAME, serverName)
            putString(KEY_DEVICE_SECRET, secretHex)
            if (!deviceId.isNullOrBlank()) putString(KEY_PAIRED_DEVICE_ID, deviceId)
        }.apply()
    }

    fun rememberEndpoint(host: String, port: Int) {
        prefs.edit().putString(KEY_LAST_HOST, host).putInt(KEY_LAST_PORT, port).apply()
    }

    fun forgetPairing() {
        prefs.edit()
            .remove(KEY_SERVER_ID)
            .remove(KEY_SERVER_NAME)
            .remove(KEY_DEVICE_SECRET)
            .remove(KEY_LAST_HOST)
            .remove(KEY_LAST_PORT)
            // The desktop's id for this phone dies with the secret it keyed. A
            // later pairing mints both again, so keeping it would authenticate as
            // a device whose secret is gone.
            .remove(KEY_PAIRED_DEVICE_ID)
            .apply()
    }

    /**
     * Drops the stored secret, the remembered PC, and the remembered endpoint,
     * but keeps the device id the desktop minted.
     *
     * This is the recovery for "the PC no longer knows this phone" (a
     * device-secret dial answered `denied`). The secret is worthless now, so it
     * goes — but the minted id must stay, because the phone is about to re-pair
     * as the *same* device:
     *
     *  - When the user approves, the core adopts the id the request asked for:
     *    the approval is armed for that id ([PairingManager::ArmForDevice]) and
     *    the token that comes back carries it, so the new secret is stored under
     *    the id the phone already had. Claiming that same id is therefore what
     *    reconnects the phone to its own record rather than creating a stranger.
     *  - A fresh random local id could be refused `busy`: the core's "another
     *    phone already holds the session" check exempts the id that currently
     *    holds the control channel, and a phone recovering from its own just-dead
     *    channel is that id.
     *
     * The remembered PC ([pairedServerId], [pairedServerName]) goes with the
     * secret it authenticated against: the PC has no record of this phone, so
     * the pairing is dead, and keeping the name made the Connection card show a
     * paired PC whose address the store could no longer supply (the endpoint
     * goes too) — a pairing the store could not back up. The successful re-pair
     * stores both again, the name and id from the token exchange.
     *
     * The nonce counter stays too: it is monotonic, and the desktop rejects a
     * nonce it has already accepted (docs §4.4), so resetting it could only
     * replay one.
     *
     * [forgetPairing] is different: it is the user-facing "forget this PC" flow,
     * where the user is discarding the pairing itself, so the minted id is
     * dropped with the secret and a later pairing mints both afresh.
     *
     * The endpoint goes too: it described the PC we could not authenticate to,
     * and the re-dial that follows re-learns a reachable one.
     */
    fun dropRejectedSecret() {
        prefs.edit()
            .remove(KEY_SERVER_ID)
            .remove(KEY_SERVER_NAME)
            .remove(KEY_DEVICE_SECRET)
            .remove(KEY_LAST_HOST)
            .remove(KEY_LAST_PORT)
            .apply()
    }

    val hasSecret: Boolean
        get() = deviceSecretHex?.hexToBytesOrNull() != null

    private fun generateDeviceId(): String =
        "android-${Auth.randomHex(4)}"

    private companion object {
        const val PREFS_NAME = "phonecam_identity"
        const val KEY_DEVICE_ID = "deviceId"

        /** The id the paired desktop minted for this phone (docs §4.2 step 4). */
        const val KEY_PAIRED_DEVICE_ID = "pairedDeviceId"
        const val KEY_DEVICE_NAME = "deviceName"
        const val KEY_DEVICE_SECRET = "deviceSecret"
        const val KEY_SERVER_ID = "serverId"
        const val KEY_SERVER_NAME = "serverName"
        const val KEY_LAST_HOST = "lastHost"
        const val KEY_LAST_PORT = "lastPort"
        const val KEY_NONCE = "clientNonce"
        const val KEY_PREVIEW_ENABLED = "previewEnabled"
    }
}
