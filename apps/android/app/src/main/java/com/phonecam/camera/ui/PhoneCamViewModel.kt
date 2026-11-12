package com.phonecam.camera.ui

import android.app.Application
import android.content.Intent
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.phonecam.camera.BuildConfig
import com.phonecam.camera.R
import com.phonecam.camera.camera.CameraCapabilityProbe
import com.phonecam.camera.data.IdentityStore
import com.phonecam.camera.protocol.Auth
import com.phonecam.camera.protocol.CameraInfo
import com.phonecam.camera.protocol.Log
import com.phonecam.camera.session.NoticeKind
import com.phonecam.camera.session.SessionIntents
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.session.connectedTransport
import com.phonecam.camera.session.rememberedTransport
import com.phonecam.camera.session.withCameras
import com.phonecam.camera.service.CameraStreamService
import com.phonecam.camera.transport.DiscoveredServer
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch

/** The facts about this phone that the diagnostics panel shows (PRD §62). */
data class IdentitySnapshot(
    val deviceId: String,
    val deviceName: String,
    val secretFingerprint: String?,
    val pairedServerName: String?,
    val pairedAddress: String?,
)

/**
 * The screen's view of the session.
 *
 * It holds no session state of its own: [state] is the service's state, so the
 * activity, the notification and the desktop can never disagree, and rotating the
 * phone — or losing the activity entirely — does not disturb a running stream.
 * Every user action becomes a [SessionIntents] intent, and nothing here touches
 * the camera, the sockets or the encoder.
 *
 * The identity facts are recomputed whenever the session state changes, because
 * pairing happens *during* a session: a fingerprint captured once at construction
 * would still read "Not paired" after a successful QR scan.
 */
class PhoneCamViewModel(application: Application) : AndroidViewModel(application) {

    private val store = IdentityStore(application)

    private val _deviceCameras = MutableStateFlow<List<CameraInfo>>(emptyList())

    /**
     * The live session state, owned by the foreground service.
     *
     * The service's state is authoritative and is what the desktop is told; the
     * one thing the view model adds is this phone's own camera list. Nothing
     * starts the service at launch — a foreground service would put a permanent
     * notification in the shade before the user asked for anything — so before
     * the first session the service's state has no cameras in it, and the screen
     * would call the camera in use "Unknown" while showing a camera picker with
     * nothing to pick. The probe below is the same one the session runs, on the
     * same no-permission `CameraManager` data, so the two cannot disagree about
     * which cameras exist; the session's list simply wins once it has one.
     */
    val state: StateFlow<SessionState> = combine(
        CameraStreamService.sessionState,
        _deviceCameras,
    ) { session, cameras ->
        if (session.capabilities.isNotEmpty()) session else session.withCameras(cameras)
    }.stateIn(viewModelScope, SharingStarted.Eagerly, SessionState())

    /** Whether a session exists at all, which the start/stop button reflects. */
    val sessionRunning: StateFlow<Boolean> = CameraStreamService.sessionRunning

    private val _logLines = MutableStateFlow<List<String>>(emptyList())

    /** The redacted diagnostics log (PRD §63). */
    val logLines: StateFlow<List<String>> = _logLines.asStateFlow()

    private val _noticeDismissed = MutableStateFlow(false)

    /** True once the user has dismissed the current substitution notice. */
    val noticeDismissed: StateFlow<Boolean> = _noticeDismissed.asStateFlow()

    private val _notificationNoteDismissed = MutableStateFlow(false)
    val notificationNoteDismissed: StateFlow<Boolean> = _notificationNoteDismissed.asStateFlow()

    /**
     * Whether the phone shows its own viewfinder while streaming.
     *
     * Read from the session state, which the service owns, rather than kept as a
     * second copy here. The flag decides which outputs the capture session is
     * built with, and that session belongs to the service: a view-model copy
     * would have shown the switch in one position while the camera had already
     * decided the other, and would have snapped back after a rotation or a
     * process restart because it was never re-read from the saved preference.
     */
    val previewEnabled: StateFlow<Boolean> = state
        .map { it.previewEnabled }
        .stateIn(viewModelScope, SharingStarted.Eagerly, store.previewEnabled)

    fun setPreviewEnabled(enabled: Boolean) {
        dispatch(SessionIntents.setPreviewEnabled(getApplication(), enabled))
    }

    private val _toast = MutableStateFlow<String?>(null)

    /** A one-shot confirmation message, or `null`. */
    val toast: StateFlow<String?> = _toast.asStateFlow()

    /**
     * Bumped when the identity store changes without the session state changing.
     *
     * [identity] is derived from the session state so that a pairing made during
     * a session updates the panel, but "Forget this PC" changes only the store:
     * it clears the paired PC while the session stays IDLE, so the session state
     * never emits and the card kept showing the PC the user had just removed
     * until the app was restarted. Bumping this counter is the smallest signal
     * that makes [identity] recompute, without inventing a second source of truth
     * for the session — the state still owns the phase and everything else.
     */
    private val _identityRevision = MutableStateFlow(0)

    /**
     * The transport of the link already announced, or `null`.
     *
     * The "Connected over the cable/Wi-Fi" toast fires when a link comes up, and
     * a reconnect returns on the same link, so this remembers what was announced
     * and suppresses a second toast for it. It is cleared when the session
     * returns to IDLE, which is what makes the next connection a fresh
     * announcement. See [announceChange].
     */
    private var _announcedTransport: String? = null

    /**
     * Whether this phone has a hardware H.264 encoder, or `null` before the probe
     * has answered.
     *
     * `null` is the third state the diagnostics panel needs: the probe is a
     * `MediaCodecList` query that runs off the main thread, so for a moment after
     * launch nothing has looked, and a two-way `false` would render "Software" —
     * a confident claim about an encoder nobody had checked. The panel says "Not
     * detected yet" until this is non-null.
     */
    private val _hasHardwareEncoder = MutableStateFlow<Boolean?>(null)
    val hasHardwareEncoder: StateFlow<Boolean?> = _hasHardwareEncoder.asStateFlow()

    /**
     * Whether the camera probe has answered yet.
     *
     * `state.capabilities` is empty both before the probe and (in principle) on a
     * phone with no cameras, and the two must not read the same: "Cameras found:
     * 0" before anything has looked is a claim the phone has not earned. This flag
     * is what tells the diagnostics panel it is the former.
     */
    private val _camerasProbed = MutableStateFlow(false)
    val camerasProbed: StateFlow<Boolean> = _camerasProbed.asStateFlow()

    val appVersion: String = BuildConfig.VERSION_NAME

    /**
     * The banner text for `SessionState.notice`, or `null` when there is nothing
     * to say or the user has already acknowledged it (PRD §13).
     */
    val notice: StateFlow<String?> = combine(state, _noticeDismissed) { session, dismissed ->
        session.notice?.takeUnless { dismissed }
    }.stateIn(viewModelScope, SharingStarted.Eagerly, null)

    /**
     * What kind of notice [notice] is, so the screen can title it correctly.
     *
     * Carried alongside the notice rather than folded into it, because the
     * heading is a translated string the screen owns (PRD §38).
     */
    val noticeKind: StateFlow<NoticeKind> = combine(state, _noticeDismissed) { session, dismissed ->
        if (dismissed) NoticeKind.Substitution else session.noticeKind
    }.stateIn(viewModelScope, SharingStarted.Eagerly, NoticeKind.Substitution)

    val identity: StateFlow<IdentitySnapshot> = combine(
        state,
        _identityRevision,
    ) { _, _ -> readIdentity() }
        .stateIn(viewModelScope, SharingStarted.Eagerly, readIdentity())

    init {
        viewModelScope.launch {
            Log.setListener { _logLines.value = Log.snapshot() }
            _logLines.value = Log.snapshot()
        }
        // Say out loud what changed.
        //
        // The screen already shows the state, but the state is a paragraph of
        // small text that the user has to read and compare against what they
        // remember. These are the four moments where that is not good enough:
        // the user pressed something and needs to know it landed, or the
        // connection changed on its own and needs to not look like a glitch.
        viewModelScope.launch {
            var previous: SessionState? = null
            state.collect { current ->
                val before = previous
                previous = current
                // The first emission is the state at launch, which is not a
                // change and should not be announced.
                if (before == null) return@collect
                announceChange(before, current)
            }
        }
        viewModelScope.launch {
            // A MediaCodecList query touches the platform codec service; keep it
            // off the main thread even though it is quick.
            val probe = CameraCapabilityProbe(getApplication())
            _hasHardwareEncoder.value = probe.hasHardwareH264Encoder()
            // The camera list is the same kind of query and is needed before the
            // session exists, so the screen can name the camera in use instead of
            // saying "Unknown" until the user starts one.
            _deviceCameras.value = runCatching { probe.probe() }.getOrDefault(emptyList())
            _camerasProbed.value = true
        }
    }

    // --------------------------------------------------------------- commands

    fun startSession() {
        Log.i("user started the session")
        dispatch(SessionIntents.start(getApplication()))
    }

    /**
     * Hands the live session a viewfinder surface, or `null` when it goes away.
     *
     * The screen owns the surface because it owns the view; the session only
     * mirrors frames into it. Passing it through here keeps the activity from
     * reaching into the service directly.
     */
    fun setPreviewSurface(surface: android.view.Surface?) {
        CameraStreamService.previewSurface(surface)
    }

    fun stopSession() {
        Log.i("user stopped the session")
        dispatch(SessionIntents.stop(getApplication()))
    }

    fun searchNow() {
        dispatch(SessionIntents.search(getApplication()))
    }

    fun connectTo(server: DiscoveredServer) {
        dispatch(
            SessionIntents.connectDiscovered(
                context = getApplication(),
                serverId = server.serverId,
                host = server.host,
                port = server.port,
            ),
        )
    }

    fun connectManually(host: String, port: Int) {
        dispatch(SessionIntents.connectManual(getApplication(), host, port))
    }

    fun connectWithToken(host: String, port: Int, token: String) {
        dispatch(SessionIntents.connectWithToken(getApplication(), host, port, token))
    }

    fun refreshCapabilities() {
        dispatch(SessionIntents.refreshCapabilities(getApplication()))
    }

    /**
     * Switches this phone's own camera.
     *
     * The desktop normally chooses the camera (§6.2), but the user holding the
     * phone can pick too; the session reports the change in its next `state`
     * message so the desktop stays in agreement.
     */
    fun selectCamera(cameraId: String) {
        Log.i("user selected camera $cameraId on the phone")
        dispatch(SessionIntents.selectCamera(getApplication(), cameraId))
    }

    /**
     * The retry affordance on the error card.
     *
     * It re-dials the remembered PC when there is one, because that is the
     * overwhelmingly common case ("the desktop app was restarting"), and falls
     * back to searching when there is nothing remembered.
     */
    fun retry() {
        val host = store.lastHost
        val port = store.lastPort
        if (host != null && port > 0) {
            connectManually(host, port)
        } else {
            searchNow()
        }
    }

    fun acknowledgeNotice() {
        _noticeDismissed.value = true
        dispatch(SessionIntents.acknowledgeNotice(getApplication()))
    }

    fun dismissNotificationNote() {
        _notificationNoteDismissed.value = true
    }

    fun clearLog() {
        Log.clear()
        _logLines.value = Log.snapshot()
    }

    fun forgetPairing() {
        store.forgetPairing()
        // Forgetting changes the store, not the session state (the phase stays
        // IDLE), so the derived [identity] has no other reason to recompute.
        _identityRevision.value += 1
        _toast.value = getApplication<Application>().getString(R.string.forgotten_message)
        Log.i("pairing forgotten by the user")
    }

    /**
     * Turns a state change into one short sentence, or nothing.
     *
     * Only transitions the user would otherwise have to go looking for are
     * announced. A toast on every state emission would be noise: the phase
     * changes on its own during a reconnect, and the stats tick rewrites the
     * whole state object every second.
     */
    private fun announceChange(before: SessionState, after: SessionState) {
        val app = getApplication<Application>()

        // Announce a switch only when the user's selection actually changed.
        // Keying on `activeCamera` alone fired on a cold launch, when the async
        // capability probe lands and turns "no camera" into the first one. See
        // [cameraSwitchedTo].
        cameraSwitchedTo(before, after)?.let { camera ->
            _toast.value = app.getString(R.string.toast_camera_switched, cameraName(app, camera))
            return
        }

        // A pairing that just completed, named from the PC's own handshake
        // response / `Welcome` (`serverId` is set only there, never on the
        // attempt). The toast used to fire the moment the dial *started*, from
        // the scanned name or — worse — a remembered one, so a code without a
        // name announced a different PC and, with neither, called a valid code
        // invalid. The link is marked announced here too, so the pairing toast is
        // not immediately followed by the "Connected over ..." one.
        if (after.serverId != null && before.serverId != after.serverId) {
            _announcedTransport = after.connectedTransport() ?: _announcedTransport
            _toast.value = app.getString(
                R.string.paired_message,
                after.serverName ?: app.getString(R.string.toast_your_pc),
            )
            return
        }

        // Announce the link only once it is actually up. `dial` sets the
        // transport string as soon as a connection starts, so keying the toast
        // off that string alone popped "Connected over the cable" for a USB dial
        // to a PC that was not there, immediately before the error. The link is
        // real only in a connected phase, and it is announced once per link:
        // `rememberedTransport` carries the transport across a reconnect so the
        // same link is not announced again.
        val connected = after.connectedTransport()
        if (connected != null && connected != _announcedTransport) {
            _announcedTransport = connected
            _toast.value = app.getString(
                if (connected == "usb") R.string.toast_connected_usb else R.string.toast_connected_wifi,
                after.serverName ?: app.getString(R.string.toast_your_pc),
            )
            return
        }
        _announcedTransport = after.rememberedTransport(_announcedTransport)

        if (before.phase != after.phase) {
            when (after.phase) {
                SessionState.Phase.STREAMING ->
                    _toast.value = app.getString(R.string.toast_streaming_started)
                SessionState.Phase.RECONNECTING ->
                    _toast.value = app.getString(R.string.toast_reconnecting)
                else -> Unit
            }
        }
    }

    /**
     * A camera's facing, because the id alone means nothing to a person.
     *
     * Reuses the same strings the camera picker uses, so the toast and the list
     * name the same thing the same way. Two names for one camera is how a user
     * concludes they switched to something they did not.
     */
    private fun cameraName(app: Application, camera: CameraInfo): String =
        when (camera.facing) {
            "front" -> app.getString(R.string.camera_front)
            "back" -> app.getString(R.string.camera_back)
            "external" -> app.getString(R.string.camera_external)
            else -> app.getString(R.string.camera_id, camera.id)
        }

    fun consumeToast() {
        _toast.value = null
    }

    fun reportInvalidQr() {
        _toast.value = getApplication<Application>().getString(R.string.error_invalid_qr)
    }

    /**
     * The user scanned a code that parsed and the dial is starting.
     *
     * Deliberately not a pairing announcement: the name in the code is optional
     * and the PC's real name only arrives with the handshake, so naming a PC here
     * is what made a code without a name announce a remembered, different one.
     * The pairing toast fires from the connected transition instead (see
     * [announceChange]); this acknowledges the scan so it is never a silent no-op.
     */
    fun reportQrDispatched() {
        _toast.value = getApplication<Application>().getString(R.string.toast_qr_dispatched)
    }

    private fun readIdentity(): IdentitySnapshot {
        val secret = store.deviceSecretHex
        val host = store.lastHost
        val port = store.lastPort
        return IdentitySnapshot(
            deviceId = store.deviceId,
            deviceName = store.deviceName,
            secretFingerprint = secret?.let { Auth.secretFingerprint(it) },
            pairedServerName = store.pairedServerName,
            pairedAddress = host?.takeIf { it.isNotBlank() }?.let { if (port > 0) "$it:$port" else it },
        )
    }

    private fun dispatch(intent: Intent) {
        // A new command resets the acknowledgement, so a notice belonging to an
        // earlier stream can never suppress the one for the current stream.
        _noticeDismissed.value = false
        SessionIntents.dispatch(getApplication(), intent)
    }
}
