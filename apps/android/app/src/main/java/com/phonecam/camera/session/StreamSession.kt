package com.phonecam.camera.session

import android.content.Context
import android.os.Build
import com.phonecam.camera.BuildConfig
import com.phonecam.camera.camera.CameraCapabilityProbe
import com.phonecam.camera.camera.CameraController
import com.phonecam.camera.camera.EncodedFrame
import com.phonecam.camera.camera.H264Encoder
import com.phonecam.camera.camera.ResolvedVideoConfig
import com.phonecam.camera.camera.VideoConfigResolver
import com.phonecam.camera.camera.VideoPreset
import com.phonecam.camera.data.IdentityStore
import com.phonecam.camera.protocol.Auth
import com.phonecam.camera.protocol.CameraInfo
import com.phonecam.camera.protocol.ChannelRole
import com.phonecam.camera.protocol.ControlCodec
import com.phonecam.camera.protocol.DesktopMessage
import com.phonecam.camera.protocol.DeviceHealth
import com.phonecam.camera.protocol.DeviceInfo
import com.phonecam.camera.protocol.Framing
import com.phonecam.camera.protocol.Log
import com.phonecam.camera.protocol.PhoneMessage
import com.phonecam.camera.protocol.Proto
import com.phonecam.camera.protocol.hexToBytesOrNull
import com.phonecam.camera.transport.DialResult
import com.phonecam.camera.transport.DiscoveredServer
import com.phonecam.camera.transport.DiscoveryClient
import com.phonecam.camera.transport.PeerConnection
import com.phonecam.camera.transport.VideoChannel
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import java.util.concurrent.atomic.AtomicLong

/** What the phone UI renders. Mirrors the states PRD Rule 9 requires. */
data class SessionState(
    val phase: Phase = Phase.IDLE,
    val serverName: String? = null,
    val serverId: String? = null,
    val host: String? = null,
    val port: Int? = null,
    val transport: String = "lan",
    val capabilities: List<CameraInfo> = emptyList(),
    val selectedCameraId: String? = null,
    val requestedResolution: String = "auto",
    val requestedFps: Int? = null,
    val requestedBitrate: Int? = null,
    val actualResolution: String? = null,
    val actualFps: Int? = null,
    val bitrate: Int = 0,
    val mirror: Boolean = false,
    val rotation: Int = 0,
    val torch: Boolean = false,
    val zoom: Float = 1f,
    val batteryPercent: Int = -1,
    val charging: Boolean = false,
    val thermal: String = "none",
    val streamFps: Double = 0.0,
    val droppedPercent: Double = 0.0,
    val screenMode: String = "keepOn",
    val qualityMode: String = "auto",
    val antiFlicker: String = "auto",
    /**
     * Whether this phone draws its own viewfinder while streaming.
     *
     * A user setting, not a session fact: it survives the stream, and it lives in
     * the session state so the service (which owns the camera), the notification
     * and the screen all read one value. When it is false the capture session is
     * built with no preview output at all rather than drawing to a hidden surface,
     * which is what makes it a power saving instead of a cosmetic one. It never
     * affects what the PC receives.
     */
    val previewEnabled: Boolean = true,
    /** A capability substitution the user must be told about (PRD §13). */
    val notice: String? = null,
    /**
     * What kind of notice [notice] is.
     *
     * [notice] usually means "your request was substituted" (PRD §13), but not
     * always: an approval instruction is a notice too. The screen needs to know
     * which, because the banner's heading is a translated string and only a
     * substitution may say "Using a supported setting". The session states the
     * kind rather than the heading because the heading belongs to the screen's
     * copy (PRD §38).
     */
    val noticeKind: NoticeKind = NoticeKind.Substitution,
    /** A human-readable failure, never a raw exception (PRD §38). */
    val error: String? = null,
    val discovered: List<DiscoveredServer> = emptyList(),
    /** The state of the search the user last started (§9 priority 2). */
    val search: SearchState = SearchState.Idle,
) {
    enum class Phase {
        IDLE,
        DISCOVERING,
        CONNECTING,
        /** Paired and reachable; not streaming yet. */
        CONNECTED,
        STREAMING,
        RECONNECTING,
        ERROR,
    }

    val isStreaming: Boolean
        get() = phase == Phase.STREAMING

    val activeCamera: CameraInfo?
        get() = capabilities.firstOrNull { it.id == selectedCameraId }
}

/**
 * What a [SessionState.notice] is telling the user.
 *
 * The notice banner has one colour and one dismiss action, but its heading is
 * copy, and copy that says "Using a supported setting" is only true of a
 * substitution. Keeping the kind explicit is what lets the screen title an
 * approval instruction correctly without a second banner design (PRD §13).
 */
enum class NoticeKind {
    /** A request was substituted with something this phone can do (PRD §13). */
    Substitution,

    /** The phone is waiting for the user to approve the connection on their PC. */
    Approval,
}

/**
 * Adopts [cameras] as this state's camera list, keeping the current selection
 * when that camera is still among them.
 *
 * The phone learns its cameras from more than one probe — the view model's, the
 * service's and the session's — and they run at different times, so each one
 * merges its result in through here rather than each deciding for itself what to
 * do with the selection. An empty [cameras] leaves the state untouched: a probe
 * that found nothing is not evidence that the earlier one was wrong.
 */
fun SessionState.withCameras(cameras: List<CameraInfo>): SessionState {
    if (cameras.isEmpty()) return this
    return copy(
        capabilities = cameras,
        selectedCameraId = selectedCameraId?.takeIf { id -> cameras.any { it.id == id } }
            ?: cameras.firstOrNull()?.id,
    )
}

/**
 * What the phone is currently able to say about the user's search for a PC.
 *
 * Discovery is silent by nature: a probe that nobody answers produces exactly the
 * same evidence as a probe that was never sent, so without this the screen can
 * only ever say "no PC found yet" and the user cannot tell a search in progress
 * from a search that finished empty (PRD §9 priority 2). The states are
 * deliberately separate from [SessionState.Phase]: a search is a bounded question
 * the user asked, while the phase describes the session as a whole.
 */
sealed interface SearchState {
    /** No search has been run, or the last result has already faded. */
    data object Idle : SearchState

    /** A search is running; the window is bounded by the session. */
    data object Searching : SearchState

    /** The search finished and [count] PCs answered. */
    data class Found(val count: Int) : SearchState

    /** The search finished and nothing answered. */
    data object NotFound : SearchState
}

/** Starts a search, which is what disables the button and shows the spinner. */
fun SessionState.searchStarted(): SessionState = copy(search = SearchState.Searching)

/**
 * Ends a running search with what discovery found while it ran.
 *
 * The count comes from [SessionState.discovered] rather than from a counter of
 * replies, because the list is already the thing the UI renders: a PC that
 * answered and was then aged out would otherwise be announced as found with
 * nothing on screen to connect to.
 */
fun SessionState.searchCompleted(): SessionState = copy(
    search = if (discovered.isEmpty()) SearchState.NotFound else SearchState.Found(discovered.size),
)

/**
 * Returns a finished search to [SearchState.Idle] so the outcome does not outlive
 * the moment it described. A search still in progress is left alone.
 */
fun SessionState.searchResultCleared(): SessionState = when (search) {
    is SearchState.Found, SearchState.NotFound -> copy(search = SearchState.Idle)
    SearchState.Idle, SearchState.Searching -> this
}

/**
 * Puts [previous] back once a search window closes, but only while this state is
 * still [SessionState.Phase.DISCOVERING]: a connection that started during the
 * window owns the phase now, and a timer must not overwrite it.
 */
fun SessionState.searchPhaseRestored(previous: SessionState.Phase): SessionState =
    if (phase == SessionState.Phase.DISCOVERING) copy(phase = previous) else this

/**
 * Records a failure, dropping any notice that was standing.
 *
 * A notice describes the moment it was posted; a failure is a later, different
 * fact, and leaving the two on screen together is how the phone showed a stale
 * "approve the connection" instruction beside the error that the approval never
 * came. Kept pure and separate from [StreamSession.fail] so the rule that a
 * failure cannot sit beside a notice is covered by a unit test rather than only
 * by the screen it was found on.
 */
fun SessionState.failed(message: String): SessionState = copy(
    phase = SessionState.Phase.ERROR,
    error = message,
    notice = null,
    noticeKind = NoticeKind.Substitution,
)

/**
 * The state a stream teardown leaves behind.
 *
 * A teardown describes the stream, not the connection: losing the picture is
 * not losing the PC, so with the control channel still open the session falls
 * back to [SessionState.Phase.CONNECTED]. But [SessionState.Phase.RECONNECTING]
 * and [SessionState.Phase.ERROR] describe what happened to the *connection*,
 * and a stream teardown must not overwrite them. A duplicate teardown — the
 * control reader reporting a close someone else had performed — used to reset
 * the retry state to IDLE, which is why the chip read "Idle" while the phone
 * was reconnecting and while the terminal error was on screen. The duplicate
 * itself is prevented by cancelling the reader before its socket is closed;
 * this rule is the second half of the same guarantee, kept pure so it is
 * testable without a live session.
 */
fun SessionState.afterStreamStopped(controlOpen: Boolean): SessionState = when (phase) {
    SessionState.Phase.RECONNECTING, SessionState.Phase.ERROR -> this
    else -> copy(phase = if (controlOpen) SessionState.Phase.CONNECTED else SessionState.Phase.IDLE)
}

/**
 * The transport a dial to [host] uses, as this phone reports it.
 *
 * USB mode is `adb reverse` (docs/protocol.md §1): the desktop core runs the
 * tunnel and tells the phone to dial `127.0.0.1` on the service port, so a
 * loopback peer means the link is the USB cable. Anything else is the local
 * network. This is a phone-side fact the phone derives — the core keeps its own
 * `transport` field — and the two are computed independently from the same rule
 * rather than one being copied from the other. Kept pure so the rule is testable
 * without a live session, which is the only way this was ever going to be
 * covered: the field used to default to `"lan"` and nothing assigned it, so every
 * USB session read "Local network".
 */
fun transportForHost(host: String): String {
    val normalized = host.trim().lowercase()
    return if (normalized == "127.0.0.1" || normalized == "localhost" || normalized == "::1") "usb"
    else "lan"
}

/**
 * The transport of a link that is actually up, or null when there is none.
 *
 * The "Connected over the cable" toast used to be keyed off the transport string
 * alone, and `dial` sets that string the moment a connection *starts*. A USB dial
 * to a PC that was not there therefore popped "Connected over the cable" and then
 * failed, announcing a link that never existed — the same class of defect as the
 * status chip that read a health state before it was true. The honest moment is
 * when the phase is CONNECTED or STREAMING, so this returns the transport only
 * then; every other phase (connecting, reconnecting, idle, error) describes an
 * attempt, not a link. Kept pure so the rule is testable without a live session.
 */
fun SessionState.connectedTransport(): String? = when {
    phase != SessionState.Phase.CONNECTED && phase != SessionState.Phase.STREAMING -> null
    transport == "usb" -> "usb"
    transport == "lan" -> "lan"
    else -> null
}

/**
 * The transport to remember for the current link, given [previous].
 *
 * A reconnect comes back on the same cable or Wi-Fi, so remembering the
 * transport is what stops the toast firing a second time when the link returns:
 * the caller announces only when `connectedTransport()` differs from this. A
 * teardown to IDLE forgets it, so the next connection is a fresh announcement.
 * Every other phase (a dial in flight, a retry, an error) keeps the previous
 * value, because it describes the same attempt rather than a new link.
 */
fun SessionState.rememberedTransport(previous: String?): String? = when {
    phase == SessionState.Phase.IDLE -> null
    else -> connectedTransport() ?: previous
}

/**
 * The screen mode actually in force, or the neutral default when none is.
 *
 * `set_screen` is a desktop request the phone applies as a window override, and
 * [com.phonecam.camera.ui.ApplyScreenMode] only applies it while the phase is
 * STREAMING. Outside a stream the phone's screen is at its normal brightness
 * whatever the desktop last asked for, so a "Phone screen: Black" row read from
 * the raw [SessionState.screenMode] kept asserting a mode that had already been
 * released — the same defect class as the status chip that read a health state
 * before it was true. The mode is kept in the state (so a restart re-applies it)
 * and this rule is only about what may be *reported*. Kept pure so it is
 * testable without a live window.
 */
fun SessionState.appliedScreenMode(): String =
    if (isStreaming) screenMode else "keepOn"

/**
 * Whether the torch is actually lit.
 *
 * The torch is part of the camera's repeating request, so it goes dark the
 * moment the stream is torn down, and the phone only ever lights it while a
 * stream is running. A row read from the raw [SessionState.torch] therefore kept
 * saying "Torch: On" after the stream ended, for a torch that was off. Kept pure
 * so it is testable without a live camera.
 */
fun SessionState.appliedTorch(): Boolean = isStreaming && torch

/**
 * Whether the streaming overlay's "Tap to show controls" hint is true.
 *
 * The hint describes exactly one interaction: in `screen_black` a blackout
 * surface covers the whole UI, and a tap is what reveals the overlay again.
 * Every other screen mode leaves the overlay fully visible, so nothing is hidden
 * behind a tap and the hint promised an interaction that does not exist. Kept
 * pure so the rule is testable without a live window.
 */
fun SessionState.overlayHintVisible(): Boolean = appliedScreenMode() == "black"

/**
 * Whether the phone's own health readings (battery, temperature) are live.
 *
 * They are sampled by the once-per-second stats tick, which runs only while a
 * stream does. Outside a stream the last sample is stale, so a present-tense
 * claim about the phone — "Your phone is warming up" — would outlive the
 * condition it described. The screen uses this to withhold the advice rather
 * than repeat a reading nobody is taking.
 */
val SessionState.hasLiveHealth: Boolean
    get() = isStreaming

/**
 * Orchestrates one phone↔desktop session.
 *
 * Owns the control connection, the video connection, the camera and the encoder,
 * and translates the desktop's commands into capability-checked camera actions.
 * Every failure path produces user-facing prose rather than an exception
 * (PRD §38, Rule 6).
 */
class StreamSession(
    private val context: Context,
    private val identity: IdentityStore,
) {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    // Seeded from the saved preference, so a phone whose owner turned the
    // viewfinder off does not show one for the first frame of every stream while
    // the setting is re-read.
    private val _state = MutableStateFlow(SessionState(previewEnabled = identity.previewEnabled))
    val state: StateFlow<SessionState> = _state.asStateFlow()

    private val capabilityProbe = CameraCapabilityProbe(context)

    /**
     * The camera list, probed once for the whole process.
     *
     * The probe walks CameraManager and MediaCodecList; the init probe here, the
     * connect-time capabilities send and the service's own startup probe all
     * want the same answer, and the view model runs a fourth of its own outside
     * this class. [CameraCapabilityProbe.cameras] runs it once and hands every
     * caller the same list. [refreshCapabilities] is the explicit way to
     * re-probe, and does so on its own instance rather than through this cache.
     */
    private fun probeCameras(): List<CameraInfo> = CameraCapabilityProbe.cameras(context)

    private val camera = CameraController(context)
    private val health = DeviceHealth(context)

    private val discovery = DiscoveryClient(
        context = context,
        scope = scope,
        deviceId = identity.deviceId,
        beaconProvider = ::phoneBeacon,
    )

    private var control: PeerConnection? = null
    private var video: PeerConnection? = null
    private var videoChannel: VideoChannel? = null
    private var encoder: H264Encoder? = null
    private var controlJob: Job? = null

    /**
     * The coroutine reading control messages from [control].
     *
     * Held so a teardown can cancel it *before* it closes the socket. The reader
     * is what turns a closed socket into [onConnectionLost], and it must be the
     * only thing that does: a close performed deliberately must not be reported
     * as a lost connection, because the duplicate report's teardown stomped the
     * retry state the deliberate one had just set.
     */
    private var controlReaderJob: Job? = null

    private var statsJob: Job? = null
    private var restartJob: Job? = null
    private var streamingJob: Job? = null

    /**
     * Consecutive camera-start failures on this connection.
     *
     * A camera error mid-stream is usually transient on the HAL level (a
     * teardown that has not finished, a sensor that is still powering down
     * after a switch), and it clears if the phone simply waits and retries.
     * Retrying at full speed is the failure mode this counter exists to stop:
     * a ~350 ms loop of open-fail-open-fail keeps the HAL wedged forever,
     * which reads on the desktop as "the stream died and I had to press Start
     * again". Each attempt steps the delay up, so a healthy camera starts on
     * the first quick retry and a genuinely stuck one reaches a visible end
     * state instead of spinning silently.
     */
    private var cameraStartFailures = 0

    /**
     * The pending reconnect attempt, held so any teardown or new connect can
     * cancel it. Without this a scheduled attempt fired after the user had
     * already disconnected or reconnected by hand, dialling a PC they had
     * deliberately left.
     */
    private var reconnectJob: Job? = null

    /**
     * The bounded wait for a pairing approval decision.
     *
     * Cancelled the moment the decision arrives, the socket drops or a new dial
     * starts, so it can never fire against a connection it no longer describes.
     */
    private var approvalTimeoutJob: Job? = null

    /**
     * The bounded window a search runs for. Replaced by the next search, and
     * cancelled when the session stops; it never outlives the user's request.
     */
    private var searchJob: Job? = null

    private val encodedFrames = AtomicLong(0)
    private val lastStatsAt = AtomicLong(System.currentTimeMillis())
    private val lastStatsFrames = AtomicLong(0)
    private val lastStatsBytes = AtomicLong(0)

    /**
     * A camera the user asked for before the capability list was ready.
     *
     * The UI offers the picker as soon as the view model's own probe lands, which
     * can be before this session's probe has finished; dropping the tap would
     * leave the user looking at a switch that did nothing.
     */
    @Volatile
    private var pendingCameraId: String? = null

    init {
        // The phone knows its own cameras before any desktop does. Probing at
        // construction — not only on connect — is what lets the UI name the
        // camera in use and offer the others while the session is still idle,
        // instead of reporting "Unknown" until the first `capabilities` send.
        // The probe reads CameraManager, which needs no permission, so this is
        // safe before the user has granted anything. It is the shared probe, so
        // connecting later does not enumerate the cameras a second time.
        scope.launch {
            // Unlike the probe inside a connection, this one runs unattended at
            // startup: an exception here would reach the default handler and take
            // the process with it, so it is contained (Rule 6).
            val probed = runCatching { probeCameras() }.getOrDefault(emptyList())
            _state.value = _state.value.withCameras(probed)
            // A camera picked before this probe finished could not be resolved
            // then, because the list it needed did not exist yet. Now it does.
            pendingCameraId?.let { requested ->
                pendingCameraId = null
                selectCameraLocally(requested)
            }
        }
    }

    /**
     * The UI's viewfinder surface, when one is on screen.
     *
     * Held rather than passed to `startStreaming`, because the viewfinder can
     * appear before a stream exists (the user is looking at the idle screen) or
     * after one is running (they scrolled it into view). Either order has to end
     * with the frames mirrored to it.
     */
    @Volatile
    private var previewSurface: android.view.Surface? = null

    /**
     * Hands the camera a viewfinder to mirror frames to, or `null` to withdraw it.
     *
     * Called by the UI as its surface comes and goes. When a stream is live the
     * capture session is rebuilt around the new output set, which keeps the camera
     * open; the encoder and the PC see no interruption.
     */
    fun setPreviewSurface(surface: android.view.Surface?) {
        previewSurface = surface
        // A surface handed to a session whose preview is off must not open one.
        // The camera is told `null` so the capture session is built without the
        // extra output; the surface itself is remembered, so turning the preview
        // back on can attach it without the screen having to recreate the view.
        camera.setPreviewSurface(if (_state.value.previewEnabled) surface else null)
    }

    /**
     * Turns this phone's own viewfinder on or off and remembers the choice.
     *
     * A user setting rather than a desktop command: it decides whether the phone
     * spends a surface, and some power and heat, on a picture nobody may be
     * looking at. It is deliberately *not* the desktop's `set_screen` (which is
     * about the panel) and it never changes what the PC receives — the encoder
     * keeps its output either way.
     *
     * Applied to the live capture session immediately when a stream is running,
     * so the switch does not require stopping the stream. That rebuild keeps the
     * camera open (see `CameraController.setPreviewSurface`), so the encoder and
     * the PC see no interruption.
     */
    fun setPreviewEnabled(enabled: Boolean) {
        if (_state.value.previewEnabled == enabled) return
        identity.previewEnabled = enabled
        _state.value = _state.value.copy(previewEnabled = enabled)
        Log.i("phone viewfinder ${if (enabled) "enabled" else "disabled"}")
        camera.setPreviewSurface(if (enabled) previewSurface else null)
    }

    @Volatile
    private var reconnectAttempts = 0

    /** Manual endpoint, set by the UI when the user types an IP. */
    @Volatile
    private var manualHost: String? = null

    @Volatile
    private var manualPort: Int? = null

    // ---------------------------------------------------------------- lifecycle

    fun start() {
        discovery.start()
        // The user has asked for a PC, so the phone advertises itself until one
        // answers. Once a connection is up the beacon has nobody left to inform
        // and is turned off (see the connect path); the receive loop stays up so
        // the desktop can still probe this phone.
        discovery.setBeaconing(true)
        scope.launch {
            discovery.servers.collect { servers ->
                _state.value = _state.value.copy(discovered = servers)
            }
        }
        Log.i("session started deviceId=${identity.deviceId}")
        autoConnectIfPossible()
    }

    fun stop() {
        stopStreaming(notifyPeer = false)
        stopStatsLoop()
        stopReconnecting()
        disconnect(notifyPeer = true)
        searchJob?.cancel()
        searchJob = null
        discovery.stop()
        scope.cancel()
    }

    /** Connects to a discovered desktop, or the remembered one (§9 priority 2). */
    fun connectTo(server: DiscoveredServer) {
        manualHost = server.host
        manualPort = server.port
        connect(server.host, server.port, null)
    }

    /** Manual IP fallback, typed by the user on the phone (§9 priority 3). */
    fun connectManually(host: String, port: Int) {
        manualHost = host
        manualPort = port
        identity.rememberEndpoint(host, port)
        connect(host, port, null)
    }

    /** QR pairing (§9 priority 1). */
    fun connectWithToken(host: String, port: Int, token: String) {
        manualHost = host
        manualPort = port
        connect(host, port, token)
    }

    fun disconnect(notifyPeer: Boolean = true) {
        stopStreaming(notifyPeer = false, keepVideoChannel = false)
        // Every teardown path stops the loops it owns, not just the socket: the
        // stats tick otherwise kept reading the battery and re-posting the
        // notification once a second after the desktop had gone.
        stopStatsLoop()
        stopReconnecting()
        controlJob?.cancel()
        controlJob = null
        // The reader goes before the socket does: closing first would make the
        // reader report a lost connection for a teardown the user asked for.
        stopControlReader()
        if (notifyPeer) {
            runCatching { control?.close() }
        }
        control = null
        // Back to looking for a PC: the beacon had been off for the duration of
        // the connection, and a phone that has just lost its desktop is exactly
        // when the user needs to see it again.
        discovery.setBeaconing(true)
        // The link is gone, so the transport it used is no longer a fact about
        // anything. Resetting it to the neutral value means the next connection is
        // a fresh announcement rather than one suppressed by an unchanged string;
        // the announcement itself waits for the phase to be connected (see
        // `connectedTransport`), so this reset never makes one fire early.
        _state.value = _state.value.copy(
            phase = SessionState.Phase.IDLE,
            transport = "none",
        )
        Log.i("session disconnected")
    }

    private fun autoConnectIfPossible() {
        scope.launch {
            // Give discovery a moment, then prefer a desktop that already knows us.
            delay(1200)
            val known = _state.value.discovered.firstOrNull { it.knowsThisDevice }
                ?: _state.value.discovered.firstOrNull { it.pairingOpen }
            if (known != null && control == null) {
                Log.i("auto-connecting to discovered desktop ${known.serverId}")
                connect(known.host, known.port, null)
                return@launch
            }

            val host = manualHost ?: identity.lastHost
            val port = manualPort ?: identity.lastPort.takeIf { it > 0 }
            if (host != null && port != null && control == null && identity.hasSecret) {
                Log.i("auto-connecting to remembered endpoint $host:$port")
                connect(host, port, null)
            }
        }
    }

    // ---------------------------------------------------------------- connect

    /**
     * A user- or UI-initiated connect.
     *
     * Starts a fresh reconnect budget and supersedes any retry already scheduled,
     * then dials. The reconnect timer deliberately does not come through here —
     * it must not reset the very counter that bounds it (see [dial]).
     */
    private fun connect(host: String, port: Int, token: String?) {
        if (controlJob?.isActive == true) return
        // A fresh connect supersedes any scheduled retry and starts a new budget.
        stopReconnecting()
        dial(host, port, token)
    }

    /** The dial itself, shared by a user connect and by the reconnect timer. */
    private fun dial(host: String, port: Int, token: String?) {
        if (controlJob?.isActive == true) return

        // A new attempt supersedes any approval wait the previous one left armed;
        // its guard only checks the phase, which a fresh dial also sets to
        // CONNECTING, so without this a stale timer could abort the new attempt.
        approvalTimeoutJob?.cancel()
        approvalTimeoutJob = null
        // It supersedes the previous attempt's reader too: a reader left on the
        // old socket would wake when that socket is closed and report a lost
        // connection against the attempt this dial is starting.
        stopControlReader()

        _state.value = _state.value.copy(
            phase = SessionState.Phase.CONNECTING,
            host = host,
            port = port,
            transport = transportForHost(host),
            error = null,
            notice = null,
            noticeKind = NoticeKind.Substitution,
        )

        controlJob = scope.launch {
            var result = PeerConnection.dialWithTokenOrSecret(
                identity = identity,
                host = host,
                port = port,
                role = ChannelRole.CONTROL,
                token = token,
            )

            // The PC does not know this phone any more.
            //
            // It happens after the desktop's registry is cleared, after a
            // reinstall, and after pairing with a different PC. The stored secret
            // is worthless, so the phone asks to be paired again — which is a
            // request the user can approve — instead of showing an error telling
            // them to approve something that was never asked for.
            //
            // Only the secret (and the stale endpoint) is dropped, not the id the
            // desktop minted: the re-pair must claim that same id so the approval
            // is armed for this device and its record is reused rather than a
            // stranger's being created (see [IdentityStore.dropRejectedSecret]).
            if (result is DialResult.CredentialsRejected && token == null) {
                identity.dropRejectedSecret()
                Log.i("the PC no longer recognises this phone; asking to pair again")
                result = PeerConnection.dialWithTokenOrSecret(
                    identity = identity,
                    host = host,
                    port = port,
                    role = ChannelRole.CONTROL,
                    token = null,
                )
            }

            handleDialResult(result, host, port, token)
        }
    }

    private suspend fun handleDialResult(
        result: DialResult,
        host: String,
        port: Int,
        token: String?,
    ) {
        when (result) {
            is DialResult.Ok -> {
                val connection = result.connection
                control = connection
                reconnectAttempts = 0
                // A desktop is connected: the beacon has nobody left to inform,
                // and broadcasting to every interface while idle, disconnected
                // and screen-off is pure cost. The receive loop stays up so a
                // probe from the desktop is still answered.
                discovery.setBeaconing(false)
                identity.rememberEndpoint(host, port)

                // A token dial returns the freshly minted device secret (§4.2).
                val body = connection.response.body
                val secret = body.jsonObject()?.get("deviceSecret")?.jsonPrimitive?.contentOrNullSafe()
                val serverId = body.jsonObject()?.get("serverId")?.jsonPrimitive?.contentOrNullSafe()
                val serverName = body.jsonObject()?.get("name")?.jsonPrimitive?.contentOrNullSafe()
                // The desktop keys the secret by the id it minted, so this must be
                // stored with it or every later connection authenticates as a
                // device the desktop has never seen (§4.2 step 4).
                val assignedId = body.jsonObject()?.get("deviceId")?.jsonPrimitive?.contentOrNullSafe()
                if (secret != null && secret.hexToBytesOrNull() != null && serverId != null) {
                    identity.rememberPairing(
                        serverId = serverId,
                        serverName = serverName ?: "PhoneCam Desktop",
                        secretHex = secret,
                        deviceId = assignedId,
                    )
                    Log.i("paired with ${serverName ?: serverId} (secret ${Auth.secretFingerprint(secret)})")
                }

                _state.value = _state.value.copy(
                    phase = SessionState.Phase.CONNECTED,
                    serverName = serverName ?: _state.value.serverName,
                    serverId = serverId ?: _state.value.serverId,
                    error = null,
                    notice = null,
                    noticeKind = NoticeKind.Substitution,
                )

                sendHello()
                sendCapabilities()
                startControlLoop()
                // The stats tick is started when the stream starts, not here:
                // it exists to report fps and bitrate, which only exist while
                // streaming.
            }

            is DialResult.ApprovalPending -> {
                // The desktop is prompting the user. Keep the socket open and wait
                // for the decision, which arrives as a control message (§4.3).
                control = result.connection
                Log.i("awaiting pairing approval on the desktop")
                _state.value = _state.value.copy(
                    phase = SessionState.Phase.CONNECTING,
                    notice = "Check your PC: approve the connection request from ${identity.deviceName}.",
                    // Not a substitution, so it gets its own heading; see
                    // [NoticeKind].
                    noticeKind = NoticeKind.Approval,
                )
                startControlLoop()
                // A prompt nobody answers is a real end state, so the wait is
                // bounded: after [APPROVAL_TIMEOUT_MS] the approval is treated as
                // a lost connection and routed into the same bounded reconnect
                // path, which either reaches the desktop or ends in the visible
                // "couldn't reach your PC" error. Without this the phone sat in
                // CONNECTING with the approval notice forever.
                approvalTimeoutJob?.cancel()
                approvalTimeoutJob = scope.launch {
                    delay(APPROVAL_TIMEOUT_MS)
                    if (isActive && _state.value.phase == SessionState.Phase.CONNECTING) {
                        Log.i("no approval after ${APPROVAL_TIMEOUT_MS}ms; giving up on this attempt")
                        // The reader goes first, and this time it is really the
                        // reader: [controlReaderJob] is the coroutine reading this
                        // socket, so cancelling it means the close below is not
                        // reported back as a lost connection. Cancelling the dial
                        // job instead — which had already finished — left the
                        // reader alive, and its second `onConnectionLost` stomped
                        // the retry state this call sets.
                        stopControlReader()
                        onConnectionLost()
                    }
                }
            }

            DialResult.Denied -> fail("The PC declined the connection. Approve the request on your PC and try again.")
            DialResult.VersionMismatch -> fail("Your PhoneCam apps need to be updated. This phone and the PC are running incompatible versions.")
            DialResult.Busy -> fail("Another phone is already connected to that PC. Disconnect it first.")
            // Only reachable when a QR token was used, since an unrecognised
            // device is handled by pairing again rather than by failing.
            DialResult.CredentialsRejected ->
                fail("This PC no longer recognises this phone. Pair again with a fresh QR code.")
            DialResult.TokenExpired -> fail("That pairing code has expired. Ask PhoneCam Desktop to show a new QR code.")
            DialResult.BadRequest -> fail("The PC didn't accept the connection. Try pairing again.")
            DialResult.UnauthorizedSource -> fail("The PC rejected the connection source. Check your network settings.")
            is DialResult.Unreachable -> {
                if (reconnectAttempts < MAX_RECONNECT_ATTEMPTS) {
                    _state.value = _state.value.copy(
                        phase = SessionState.Phase.RECONNECTING,
                        error = null,
                    )
                }
                // A PC with Wi-Fi and Ethernet advertises a beacon per interface,
                // and on some networks one of those addresses is not routable from
                // the phone (the LAN test hit exactly this: the phone reached
                // 192.168.100.53 but not .5). Retrying the same dead address until
                // the budget runs out never tries the one that works, so an
                // unreachable dial moves to another address for the same PC.
                val nextHost = alternativeHost(host) ?: host
                if (nextHost != host) {
                    Log.i("trying another address for the same PC: $nextHost")
                }
                scheduleReconnect(
                    nextHost,
                    port,
                    token,
                    "PhoneCam couldn't reach your PC. Check that it is on and on the same network, then try again.",
                )
            }
            is DialResult.Failed -> fail(result.message)
        }
    }

    private fun fail(message: String, code: String = "stream_failed") {
        // A failure supersedes any standing notice: the two describe different
        // moments, and a notice left beside the error that ended it reads as a
        // contradiction. See [SessionState.failed].
        _state.value = _state.value.failed(message)
        Log.w("session error: $message")
        reportFailure(code, message)
    }

    private val controlWriteLock = Any()

    private fun sendControlMessage(message: PhoneMessage): Boolean {
        val connection = control ?: return false
        return runCatching {
            val encoded = ControlCodec.encode(message)
            synchronized(controlWriteLock) {
                Framing.writeControl(connection.output, encoded)
            }
            true
        }.onFailure {
            Log.d("control send ${message.javaClass.simpleName} failed: ${it.javaClass.simpleName}")
        }.isSuccess
    }

    /**
     * Tells the desktop why a stream could not start, or why it stopped.
     *
     * Every abort used to be local: `fail()` only touched this phone's own state,
     * and the paths that call `stopStreaming(notifyPeer = false)` deliberately say
     * nothing at all. The desktop was left watching a video channel close with no
     * explanation, which is what "the log has no error" meant. The reason can only
     * travel on the control channel, so it goes out on that one whenever it is
     * still open; a phone that has lost the PC has nothing to report to.
     */
    private fun reportFailure(code: String, message: String) {
        sendControlMessage(PhoneMessage.Error(code, message))
    }

    /**
     * Schedules the next reconnect attempt, up to [MAX_RECONNECT_ATTEMPTS].
     *
     * The loop used to be infinite: a desktop switched off left the phone
     * re-dialling forever, and the counter that bounded the backoff reset only
     * on a successful dial. Past the ceiling the attempt is abandoned for a
     * terminal error carrying [finalMessage], which the user sees and can retry
     * from — an honest end state instead of silent, endless retrying.
     */
    /**
     * Another address for the same desktop, when the current one is unreachable.
     *
     * The desktop advertises a beacon per local address, so a PC with Wi-Fi and
     * Ethernet appears twice with one `serverId`. A phone may be able to reach one
     * and not the other; this returns the alternative, preferring an address whose
     * beacon says the desktop already knows this phone. Returns null when there is
     * no other address, and the caller keeps the current one.
     */
    private fun alternativeHost(currentHost: String): String? {
        val knownId = identity.pairedServerId
        // Every address the PC has advertised, not just the latest beacon's: the
        // discovery list keeps one entry per server, so the other interface would
        // otherwise be invisible.
        discovery.hostsFor(knownId).firstOrNull { it != currentHost }?.let { return it }
        val candidates = _state.value.discovered.filter {
            it.host != currentHost && (knownId == null || it.serverId == knownId)
        }
        return (candidates.firstOrNull { it.knowsThisDevice } ?: candidates.firstOrNull())?.host
    }

    private fun scheduleReconnect(host: String, port: Int, token: String?, finalMessage: String) {
        if (reconnectAttempts >= MAX_RECONNECT_ATTEMPTS) {
            reconnectAttempts = 0
            // The phone is on its own again and the user needs to see it on the
            // desktop, so resume the beacon the connection had switched off.
            discovery.setBeaconing(true)
            fail(finalMessage)
            return
        }
        _state.value = _state.value.copy(
            phase = SessionState.Phase.RECONNECTING,
            error = null,
            notice = null,
            noticeKind = NoticeKind.Substitution,
        )
        val delayMs = backoffDelay()
        reconnectJob?.cancel()
        reconnectJob = scope.launch {
            // The approval wait, if any, has been decided or abandoned by now; a
            // notice explaining it must not survive into the retry state.
            _state.value = _state.value.copy(
                phase = SessionState.Phase.RECONNECTING,
                error = null,
                notice = null,
                noticeKind = NoticeKind.Substitution,
            )
            Log.i("reconnecting in ${delayMs}ms (attempt ${reconnectAttempts + 1}/$MAX_RECONNECT_ATTEMPTS)")
            // Probe while retrying. The phone may never hear the desktop's
            // broadcast beacons (an AP that isolates clients, a firewall that
            // drops UDP broadcast), but a §3.3 probe is answered unicast, which
            // arrives on the same receive path and fills the address list the
            // unreachable-dial fallback needs. Without it, a phone that dialled a
            // dead address could never learn the PC's working one.
            discovery.probe()
            delay(delayMs)
            reconnectAttempts++
            dial(host, port, token)
        }
    }

    /**
     * Cancels a scheduled reconnect and clears the attempt budget.
     *
     * Called from every teardown and from every fresh dial, so a retry cannot
     * outlive the intent that created it or fire after the user has reconnected
     * by another route.
     */
    private fun stopReconnecting() {
        reconnectJob?.cancel()
        reconnectJob = null
        // The approval wait bounds the same attempt the budget counts, so it is
        // cleared here too: a fresh dial or teardown must not leave it to fire
        // against a connection it no longer describes. The reader belongs to the
        // same attempt, and a teardown that clears the budget must not leave it
        // armed to report a loss the teardown is about to handle itself.
        approvalTimeoutJob?.cancel()
        approvalTimeoutJob = null
        stopControlReader()
        reconnectAttempts = 0
    }

    private fun backoffDelay(): Long {
        val steps = listOf(1000L, 2000L, 3000L, 4000L, 6000L)
        return steps[reconnectAttempts.coerceIn(0, steps.size - 1)]
    }

    // ---------------------------------------------------------------- outbound

    private fun sendHello() {
        sendControlMessage(
            PhoneMessage.Hello(
                protocol = Proto.VERSION,
                deviceId = identity.deviceId,
                deviceName = identity.deviceName,
                model = Build.MODEL,
                androidVersion = Build.VERSION.SDK_INT,
                appVersion = BuildConfig.VERSION_NAME,
            ),
        )
    }

    private fun sendCapabilities() {
        val cameras = probeCameras()
        _state.value = _state.value.copy(
            capabilities = cameras,
            selectedCameraId = _state.value.selectedCameraId ?: cameras.firstOrNull()?.id,
        )
        sendControlMessage(
            PhoneMessage.Capabilities(
                device = DeviceInfo(
                    model = Build.MODEL,
                    androidVersion = Build.VERSION.SDK_INT,
                    appVersion = BuildConfig.VERSION_NAME,
                ),
                cameras = cameras,
            ),
        )
    }

    private fun sendState() {
        val current = _state.value
        sendControlMessage(
            PhoneMessage.State(
                streaming = current.isStreaming,
                camera = current.activeCamera?.facing ?: "unknown",
                resolution = current.requestedResolution,
                fps = current.requestedFps ?: 0,
                actualResolution = current.actualResolution ?: "",
                actualFps = current.actualFps ?: 0,
                bitrate = current.bitrate,
                mirror = current.mirror,
                rotation = current.rotation,
                torch = current.torch,
                zoom = current.zoom,
                battery = current.batteryPercent,
                charging = current.charging,
                thermal = current.thermal,
                antiFlicker = current.antiFlicker,
            ),
        )
    }

    private fun startControlLoop() {
        val connection = control ?: return
        // A second reader on the same socket would race the first for frames;
        // one reader per control connection, replaced only by cancelling.
        stopControlReader()
        controlReaderJob = scope.launch {
            try {
                while (isActive && connection.isOpen) {
                    val payload = Framing.readControl(connection.input)
                    val message = ControlCodec.decodeDesktop(payload)
                    if (message == null) {
                        // Forward compatibility: an unknown command is skipped, not fatal.
                        Log.d("ignoring unknown control message (${payload.size} bytes)")
                        continue
                    }
                    handleDesktopMessage(message)
                }
            } catch (t: Throwable) {
                // Only a reader that is still wanted may report the loss; one
                // cancelled by a deliberate teardown must stay silent.
                if (isActive) {
                    Log.w("control channel closed: ${t.javaClass.simpleName}")
                    onConnectionLost()
                }
            }
        }
    }

    /**
     * Stops the control reader before its socket is closed.
     *
     * Every deliberate teardown calls this first, so the only [onConnectionLost]
     * that runs is the one the teardown itself performs. Cancelling a coroutine
     * blocked in a socket read does not interrupt the read on its own — the
     * close that follows is what releases it — but it clears `isActive`, which
     * is the reader's guard against reporting a close it did not experience.
     */
    private fun stopControlReader() {
        controlReaderJob?.cancel()
        controlReaderJob = null
    }

    private fun startStatsLoop() {
        statsJob?.cancel()
        statsJob = scope.launch {
            while (isActive) {
                delay(STATS_INTERVAL_MS)
                publishStats()
            }
        }
    }

    /**
     * Stops the once-per-second stats tick.
     *
     * Cancelled on every teardown path: left running, the tick kept waking the
     * session after the desktop had gone, reading the battery and thermal status,
     * copying the whole [SessionState] and re-posting the notification forever.
     */
    private fun stopStatsLoop() {
        statsJob?.cancel()
        statsJob = null
    }

    private fun publishStats() {
        val now = System.currentTimeMillis()
        val elapsed = now - lastStatsAt.getAndSet(now)
        val frames = encodedFrames.get()
        val delta = frames - lastStatsFrames.getAndSet(frames)

        val fps = if (elapsed > 0) delta * 1000.0 / elapsed else 0.0
        val channelStats = videoChannel?.stats
        val dropPercent = channelStats?.dropPercent ?: 0.0
        val healthSnapshot = health.snapshot()

        val bitrate = if (elapsed > 0 && channelStats != null) {
            // Delta bytes per interval converted to bits per second.
            val prevBytes = lastStatsBytes.getAndSet(channelStats.bytesSent)
            val deltaBytes = (channelStats.bytesSent - prevBytes).coerceAtLeast(0L)
            (deltaBytes * 8_000L / elapsed).toInt()
        } else {
            0
        }

        _state.value = _state.value.copy(
            streamFps = fps,
            droppedPercent = dropPercent,
            batteryPercent = healthSnapshot.batteryPercent,
            charging = healthSnapshot.charging,
            thermal = healthSnapshot.thermal,
        )

        sendControlMessage(
            PhoneMessage.Stats(
                fps = fps,
                bitrate = bitrate,
                dropped = dropPercent,
                encodedFrames = frames,
                // Always 0: the round trip is measured by the desktop,
                // which is the only side that can time the return leg
                // (docs/protocol.md §11). The field is part of the frozen
                // message and stays so the shape does not change.
                rttUs = 0,
                thermal = healthSnapshot.thermal,
            ),
        )
    }

    // ---------------------------------------------------------------- inbound

    private fun handleDesktopMessage(message: DesktopMessage) {
        when (message) {
            is DesktopMessage.Welcome -> {
                Log.i("desktop welcome from ${message.serverName} v${message.serverVersion}")
                _state.value = _state.value.copy(serverName = message.serverName)
            }

            is DesktopMessage.SetCamera -> selectCamera(message.camera)
            is DesktopMessage.SetVideo -> applyVideoRequest(message)
            is DesktopMessage.SetImage -> applyImageRequest(message)
            is DesktopMessage.SetTorch -> applyTorch(message.enabled)
            is DesktopMessage.SetAntiFlicker -> {
                camera.setAntibandingMode(message.mode)
                _state.value = _state.value.copy(antiFlicker = message.mode)
                sendState()
            }
            is DesktopMessage.SetScreen -> {
                _state.value = _state.value.copy(screenMode = message.mode)
                sendState()
            }
            is DesktopMessage.SetQualityMode -> {
                _state.value = _state.value.copy(qualityMode = message.mode)
                sendState()
            }

            DesktopMessage.StartStream -> startStreaming(force = true)
            DesktopMessage.StopStream -> stopStreaming(notifyPeer = false)

            is DesktopMessage.Ping -> sendPong(message.serverNonce)

            is DesktopMessage.PairApproved -> {
                // The desktop approved this phone; re-dial with the one-time
                // token to exchange it for a device secret (§4.3).
                Log.i("pairing approved; re-dialling with token")
                val host = _state.value.host
                val port = _state.value.port
                // The re-dial is deliberate, so the reader on this socket must
                // not report the close as a loss — that would schedule a retry
                // against the token dial this path is about to make.
                stopControlReader()
                runCatching { control?.close() }
                control = null
                if (host != null && port != null) {
                    connect(host, port, message.token)
                }
            }

            is DesktopMessage.PairDenied -> fail("Connection declined on the PC. Approve the request to continue.")

            is DesktopMessage.Error -> {
                Log.w("desktop reported ${message.code}")
                _state.value = _state.value.copy(error = message.message)
            }
        }
    }

    private fun sendPong(nonce: Long) {
        sendControlMessage(PhoneMessage.Pong(nonce))
    }

    /** The camera a `set_camera` value names, or `null` when this phone has no such camera. */
    private fun cameraFor(facingOrId: String): CameraInfo? {
        val cameras = _state.value.capabilities
        return cameras.firstOrNull { it.id == facingOrId }
            ?: cameras.firstOrNull { it.facing == facingOrId }
    }

    private fun selectCamera(facingOrId: String) {
        val target = cameraFor(facingOrId)
        if (target == null) {
            Log.w("requested camera '$facingOrId' is not present on this phone")
            _state.value = _state.value.copy(error = "That camera isn't available on this phone.")
            return
        }

        // Re-selecting the camera already in use must not restart the stream: a
        // tap on the selected icon would otherwise blink the picture on the PC.
        if (target.id == _state.value.selectedCameraId) {
            _state.value = _state.value.copy(error = null)
            return
        }
        val wasStreaming = _state.value.isStreaming ||
            _state.value.phase == SessionState.Phase.CONNECTING ||
            streamingJob?.isActive == true ||
            restartJob?.isActive == true
        restartJob?.cancel()
        restartJob = null

        // A switch that lands on the same stream configuration is a change of
        // capture source, not a new stream: the encoder keeps running, so its
        // input Surface, its codec configuration and its presentation clock all
        // survive. The desktop therefore sees the same SPS/PPS and never rebuilds
        // its decoder. Tearing the encoder down and starting another one is what
        // used to cost a quarter second of deliberate delay plus a codec start,
        // and is what the camera switch used to look like on the PC: a freeze.
        val liveSurface = encoder?.surface?.takeIf { it.isValid }
        val swapInPlace = wasStreaming && liveSurface != null && switchingKeepsConfiguration(target.id)

        _state.value = _state.value.copy(
            selectedCameraId = target.id,
            error = null,
            // Front camera defaults to mirrored, rear to not (PRD §23).
            mirror = target.facing == "front",
            zoom = 1f,
            torch = false,
        )
        sendState()

        if (!wasStreaming) return

        if (swapInPlace) {
            val surface = liveSurface!!
            Log.i("switching to camera ${target.id} in place; the encoder and the video channel stay up")
            val requestedResolution = _state.value.requestedResolution
            val requestedFps = _state.value.requestedFps
            val requestedBitrate = _state.value.requestedBitrate
            val preview = if (_state.value.previewEnabled && previewSurface?.isValid == true) previewSurface else null
            val targetId = target.id
            // Off the control channel's reader thread: closing the old camera
            // device blocks for a couple of hundred milliseconds, and while that
            // thread is blocked the phone cannot answer the desktop's pings —
            // which reads on the PC as exactly the freeze the switch is trying
            // to avoid.
            scope.launch {
                camera.start(
                    cameraId = targetId,
                    targetSurface = surface,
                    requestedResolution = requestedResolution,
                    requestedFps = requestedFps,
                    requestedBitrate = requestedBitrate,
                    onStarted = { applied: ResolvedVideoConfig ->
                        scope.launch {
                            cameraStartFailures = 0
                            val appliedRes = "${applied.width}x${applied.height}"
                            _state.value = _state.value.copy(
                                phase = SessionState.Phase.STREAMING,
                                actualResolution = appliedRes,
                                actualFps = applied.fps,
                                bitrate = applied.bitrate,
                                notice = applied.notice,
                                noticeKind = NoticeKind.Substitution,
                            )
                            sendState()
                            // The new sensor starts on a delta frame by default; an IDR
                            // here gives the desktop a clean reference immediately.
                            encoder?.requestKeyframe()
                        }
                    },
                    onError = { message ->
                        scope.launch { retryCameraStart(message) }
                    },
                    previewSurface = preview,
                    // The encoder and the channel are staying up; so does the camera
                    // thread, otherwise the switch pays a thread teardown it does not
                    // need before it can even open the other sensor.
                    keepThread = true,
                )
            }
            return
        }

        stopStreaming(notifyPeer = false, keepVideoChannel = true)
        restartJob = scope.launch {
            delay(VIDEO_RESTART_DELAY_MS)
            if (isActive) startStreaming()
        }
    }

    /**
     * Whether [cameraId] would resolve to the stream that is already running.
     *
     * Only then can the capture source be swapped underneath the live encoder:
     * a different size or rate needs a new codec configuration, which is a real
     * restart with a new SPS.
     */
    private fun switchingKeepsConfiguration(cameraId: String): Boolean {
        val current = _state.value.actualResolution ?: return false
        val fps = _state.value.actualFps ?: return false
        val parsed = VideoPreset.parseResolution(current) ?: return false
        val resolved = runCatching {
            VideoConfigResolver.resolve(
                requestedResolution = _state.value.requestedResolution,
                requestedFps = _state.value.requestedFps,
                requestedBitrate = _state.value.requestedBitrate,
                supportedSizes = camera.supportedSizes(cameraId),
                supportedFps = camera.supportedFrameRates(cameraId),
                highSpeedSizes = camera.highSpeedSizes(cameraId),
            )
        }.getOrNull() ?: return false
        return resolved.width == parsed.first &&
            resolved.height == parsed.second &&
            resolved.fps == fps
    }

    private fun scheduleStreamRestart(delayMs: Long = VIDEO_RESTART_DELAY_MS) {
        restartJob?.cancel()
        restartJob = scope.launch {
            delay(delayMs)
            if (isActive && !_state.value.isStreaming) startStreaming()
        }
    }

    /**
     * Retry a camera that failed mid-stream, with a stepped delay.
     *
     * The first retries are quick because the common case (a camera switch
     * racing the previous sensor's teardown) clears in well under a second.
     * Each further failure steps the wait up and, past [MAX_CAMERA_START_FAILURES],
     * gives up with the original message: a camera that never comes back is a
     * real end state the user must see, not a loop to hide.
     */
    private fun retryCameraStart(message: String) {
        if (cameraStartFailures >= MAX_CAMERA_START_FAILURES) {
            cameraStartFailures = 0
            stopStreaming(notifyPeer = false)
            fail(message)
            return
        }
        val delayMs = CAMERA_RETRY_DELAYS_MS[
            cameraStartFailures.coerceAtMost(CAMERA_RETRY_DELAYS_MS.size - 1),
        ]
        cameraStartFailures++
        Log.i("camera start failed ($message); retry $cameraStartFailures/$MAX_CAMERA_START_FAILURES in ${delayMs}ms")
        stopStreaming(notifyPeer = false, keepVideoChannel = true)
        scheduleStreamRestart(delayMs)
    }

    private fun applyVideoRequest(message: DesktopMessage.SetVideo) {
        val requestedFps = message.fps.takeIf { it > 0 }
        _state.value = _state.value.copy(
            requestedResolution = message.resolution,
            requestedFps = requestedFps,
            requestedBitrate = message.bitrate,
        )
        sendState()

        if (!_state.value.isStreaming) return
        val cameraId = _state.value.selectedCameraId ?: return
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = message.resolution,
            requestedFps = requestedFps,
            requestedBitrate = message.bitrate,
            supportedSizes = camera.supportedSizes(cameraId),
            supportedFps = camera.supportedFrameRates(cameraId),
            highSpeedSizes = camera.highSpeedSizes(cameraId),
        )
        val actualParsed = _state.value.actualResolution?.let {
            VideoPreset.parseResolution(it)
        }
        val sameResolution = actualParsed != null &&
            resolved.width == actualParsed.first &&
            resolved.height == actualParsed.second

        if (sameResolution) {
            if (resolved.fps != _state.value.actualFps) {
                camera.setFrameRate(resolved.fps)
            }
            encoder?.updateBitrate(resolved.bitrate)
            _state.value = _state.value.copy(
                actualFps = resolved.fps,
                bitrate = resolved.bitrate,
                notice = resolved.notice,
                // A substitution, so it goes back to the default heading.
                noticeKind = NoticeKind.Substitution,
            )
            sendState()
        } else {
            stopStreaming(notifyPeer = false, keepVideoChannel = true)
            scheduleStreamRestart()
        }
    }

    private fun applyImageRequest(message: DesktopMessage.SetImage) {
        _state.value = _state.value.copy(
            mirror = message.mirror,
            rotation = message.rotation,
        )
        val cameraId = _state.value.selectedCameraId
        if (message.zoom > 0f && message.zoom != _state.value.zoom && cameraId != null) {
            camera.setZoom(cameraId, message.zoom)
            // CameraController clamps to the sensor's real maximum. Report the
            // value that is actually applied, not the value the desktop asked for.
            _state.value = _state.value.copy(zoom = camera.zoomFactor)
        }
        sendState()
    }

    private fun applyTorch(enabled: Boolean) {
        val active = _state.value.activeCamera
        when {
            active == null -> {
                _state.value = _state.value.copy(error = "No camera is selected, so the torch cannot be changed.")
            }
            !active.torch -> {
                _state.value = _state.value.copy(
                    torch = false,
                    error = "This camera does not have a torch.",
                )
            }
            else -> {
                camera.setTorch(enabled)
                _state.value = _state.value.copy(torch = enabled, error = null)
            }
        }
        sendState()
    }




    private fun onConnectionLost() {
        val wasStreaming = _state.value.isStreaming
        // A socket that dropped while the phone was still connecting is the same
        // event as one that dropped mid-stream: the PC is gone. It used to be
        // ignored, so a phone waiting for pairing approval whose desktop closed
        // the socket sat in CONNECTING forever behind a stale notice.
        val wasConnecting = _state.value.phase == SessionState.Phase.CONNECTING
        // The reader is the thing that reports losses, so a loss handled here is
        // the last one this connection gets: leaving a reader armed on the dead
        // socket would let it report the same loss again against the retry state
        // below. Cancelling a reader that is itself calling this is harmless —
        // it has nothing left to run — and cancelling one armed by a teardown is
        // exactly the point.
        stopControlReader()
        stopStreaming(notifyPeer = false)
        // The socket is gone, so the stats tick has nothing to report to and the
        // reconnect timer is about to be re-armed; stop both before deciding.
        stopStatsLoop()
        runCatching { control?.close() }
        control = null

        if (wasStreaming || wasConnecting || _state.value.phase == SessionState.Phase.CONNECTED) {
            val host = _state.value.host
            val port = _state.value.port
            if (host != null && port != null) {
                scheduleReconnect(
                    host,
                    port,
                    null,
                    "Lost the connection to your PC and couldn't get it back. Make sure it is on and on the same network, then try again.",
                )
            } else {
                fail("Connection lost. Reconnect to continue.")
            }
        }
    }

    // ---------------------------------------------------------------- streaming

    private fun startStreaming(force: Boolean = false) {
        val currentChannel = videoChannel
        val currentConn = video
        val isChannelHealthy = _state.value.isStreaming &&
            currentChannel != null && currentChannel.isRunning &&
            currentConn != null && currentConn.isOpen

        if (isChannelHealthy && !force) {
            Log.i("startStreaming: already streaming with healthy video channel; requesting keyframe")
            encoder?.requestKeyframe()
            return
        }

        if (streamingJob?.isActive == true && !force) {
            Log.i("startStreaming: already connecting; ignoring duplicate request")
            return
        }

        if (_state.value.isStreaming || streamingJob?.isActive == true || !isChannelHealthy) {
            Log.i("startStreaming: resetting previous stream state (force=$force, isChannelHealthy=$isChannelHealthy)")
            stopStreaming(notifyPeer = false, keepVideoChannel = !force && isChannelHealthy)
        }

        restartJob?.cancel()
        restartJob = null
        // A start the user or the desktop asked for is a fresh attempt: the
        // stepped-retry budget belongs to the failures it is about to replace.
        cameraStartFailures = 0
        Log.i("start streaming requested host=${_state.value.host} port=${_state.value.port} camera=${_state.value.selectedCameraId} force=$force")
        val host = _state.value.host ?: return
        val port = _state.value.port ?: return
        val cameraId = _state.value.selectedCameraId ?: run {
            fail("No camera is available on this phone.")
            return
        }
        // One start at a time. `start_stream` legitimately arrives more than once
        // (the desktop retries, and switching the virtual camera on auto-starts
        // the stream), and the guard above only catches a start that has already
        // reached STREAMING. Every earlier attempt was still in CONNECTING when
        // the next one arrived, so each launched its own dial: two video channels
        // opened within milliseconds, the second replaced the first in the core,
        // and the loser's teardown closed the winner's socket. The channel opened
        // and closed again, the phase never left CONNECTING, and neither end
        // reported an error. Tearing the in-flight attempt down before dialling
        // keeps the newest request authoritative and leaves nothing half-open.
        // In-flight attempts are protected from cancellation above
        _state.value = _state.value.copy(
            phase = SessionState.Phase.CONNECTING,
            error = null,
            notice = null,
            noticeKind = NoticeKind.Substitution,
        )
        streamingJob = scope.launch {
            Log.i("opening video channel")
            // The connection this attempt opens is held locally as well as in the
            // shared field.
            //
            // The fields are shared by every attempt, and a cancelled attempt used
            // to write them anyway: it assigned `video` and `videoChannel` to its
            // own connection *after* the replacement attempt had already assigned
            // them to a newer one, so the newer connection's reference was lost
            // and its socket was never closed. Two channels then stayed open and
            // the core saw the older one win. Holding the connection here lets the
            // `finally` below close exactly what this attempt opened and nothing
            // else, whether it finished, failed, or was cancelled mid-dial.
            var attemptConnection: PeerConnection? = null
            var attemptChannel: VideoChannel? = null
            // Set only once the camera has taken ownership of the encoder surface,
            // which is the point where the session, not this attempt, owns the
            // channel. Every earlier exit is a failure and must clean up after
            // itself; without this flag the failure paths leaked the socket,
            // because the shared fields still pointed at it and so the `finally`
            // could not tell "still current" from "abandoned".
            var handedOff = false
            try {
                // A camera switch changes which sensor feeds the encoder, not
                // which socket carries the video. Dialling a second channel here
                // closes the first, which the desktop reads as the stream
                // ending: it drops the picture to the placeholder and rebuilds
                // the decoder, and that is the three-second black frame users
                // see when flipping between cameras. Reusing the open channel
                // makes the switch a re-encode instead of a reconnect.
                val openChannel = videoChannel
                val openConnection = video
                val canReuse = !force &&
                    openChannel != null && openChannel.isRunning &&
                    openConnection != null && openConnection.isOpen

                val channel: VideoChannel
                if (canReuse) {
                    Log.i("reusing the open video channel for the camera switch")
                    channel = openChannel!!
                } else {
                    val videoResult = PeerConnection.dialAsKnownDevice(
                        identity = identity,
                        host = host,
                        port = port,
                        role = ChannelRole.VIDEO,
                    )
                    if (videoResult !is DialResult.Ok) {
                        Log.w("video channel rejected: ${videoResult.javaClass.simpleName}")
                        val message = when (videoResult) {
                            DialResult.Busy -> "Another phone is already connected to that PC."
                            is DialResult.Unreachable -> "Lost the connection to the PC while starting the camera."
                            else -> "The PC wouldn't open the video channel. Try again."
                        }
                        fail(message)
                        return@launch
                    }
                    Log.i("video channel handshake accepted")

                    val videoConnection = videoResult.connection
                    attemptConnection = videoConnection
                    video = videoConnection
                    val opened = VideoChannel(
                        connection = videoConnection,
                        onClosed = { error ->
                            scope.launch {
                                Log.w("video channel closed: ${error?.javaClass?.simpleName ?: "EOF"}")
                                if (video === videoConnection) {
                                    video = null
                                    videoChannel = null
                                }
                                if (_state.value.isStreaming && control?.isOpen == true) {
                                    Log.i("video channel lost while streaming; restarting video stream")
                                    startStreaming(force = true)
                                } else if (control?.isOpen != true) {
                                    onConnectionLost()
                                }
                            }
                        },
                    ).also { it.start() }
                    attemptChannel = opened
                    videoChannel = opened
                    channel = opened
                    Log.i("video writer started")
                }

                val sizes = camera.supportedSizes(cameraId)
                val fps = camera.supportedFrameRates(cameraId)
                Log.i("camera capabilities for $cameraId: ${sizes.size} sizes, fps=${fps.joinToString()}")
                if (sizes.isEmpty()) {
                    Log.e("camera $cameraId disappeared before streaming")
                    fail("The selected camera is no longer available. Refresh cameras and try again.")
                    return@launch
                }
                val resolved = runCatching {
                    VideoConfigResolver.resolve(
                        requestedResolution = _state.value.requestedResolution,
                        requestedFps = _state.value.requestedFps,
                        requestedBitrate = _state.value.requestedBitrate,
                        supportedSizes = sizes,
                        supportedFps = fps,
                        highSpeedSizes = camera.highSpeedSizes(cameraId),
                    )
                }.getOrElse { error ->
                    Log.e("camera configuration failed", error)
                    fail("This camera cannot start with the selected video settings.")
                    return@launch
                }

                val h264 = H264Encoder(
                    width = resolved.width,
                    height = resolved.height,
                    fps = resolved.fps,
                    bitrate = resolved.bitrate,
                    onFrame = { frame: EncodedFrame ->
                        encodedFrames.incrementAndGet()
                        channel.submit(frame)
                    },
                    onError = { message ->
                        scope.launch { fail(message) }
                    },
                )
                val surface = h264.start()
                if (surface == null) {
                    // `H264Encoder.start` reports the reason through `onError`,
                    // which runs on this scope rather than inline. Tearing down
                    // first and failing here makes the outcome deterministic: the
                    // teardown used to land after `fail` and reset the phase to
                    // CONNECTED, so an encoder that refused to start looked exactly
                    // like a stream that had never been asked for.
                    stopStreaming(notifyPeer = false)
                    fail("Couldn't start video encoding on this phone.", code = "encoder_failed")
                    return@launch
                }
                Log.i("H.264 encoder surface created")
                encoder = h264

                // From here the session owns the channel: `camera.start` has the
                // surface and the encoder is live, so a later teardown must go
                // through `stopStreaming` rather than this attempt's `finally`.
                handedOff = true

                val encWidth = h264.configuredWidth
                val encHeight = h264.configuredHeight
                val encFps = h264.configuredFps
                val encBitrate = h264.configuredBitrate
                val wasSubstituted = h264.isSubstituted

                camera.start(
                    cameraId = cameraId,
                    targetSurface = surface,
                    requestedResolution = "${encWidth}x${encHeight}",
                    requestedFps = encFps,
                    requestedBitrate = encBitrate,
                    // Whatever the UI has on screen right now. A viewfinder that
                    // appears later is attached through `setPreviewSurface`, which
                    // rebuilds the session without reopening the camera.
                    previewSurface = if (_state.value.previewEnabled && previewSurface?.isValid == true) previewSurface else null,
                    onStarted = { applied: ResolvedVideoConfig ->
                        scope.launch {
                            cameraStartFailures = 0
                            val appliedRes = "${applied.width}x${applied.height}"
                            val subNotice = if (wasSubstituted && applied.notice == null) {
                                "Using ${encWidth}x${encHeight} (hardware encoder limit)."
                            } else {
                                applied.notice
                            }
                            _state.value = _state.value.copy(
                                phase = SessionState.Phase.STREAMING,
                                actualResolution = appliedRes,
                                actualFps = applied.fps,
                                bitrate = applied.bitrate,
                                notice = subNotice,
                                // A substitution, so it goes back to the default heading.
                                noticeKind = NoticeKind.Substitution,
                            )
                            // The stats tick is a property of the stream, not of
                            // the connection: it starts here and is stopped by
                            // `stopStreaming`, so a camera switch (which stops
                            // and restarts the stream) leaves exactly one tick.
                            startStatsLoop()
                            sendState()
                            encoder?.requestKeyframe()
                        }
                    },
                    onError = { message ->
                        scope.launch { retryCameraStart(message) }
                    },
                )
            } finally {
                // Close what this attempt opened, unless it handed ownership over.
                // A cancelled or failed attempt reaches here with its socket open
                // and no other owner, which is exactly the leak that left two video
                // channels alive and made the core pick the wrong one.
                if (!handedOff) {
                    // Only clear the shared fields when they still point at this
                    // attempt's connection: a newer attempt may already have
                    // replaced them, and clearing those would abandon *its* socket.
                    if (video === attemptConnection) {
                        video = null
                        videoChannel = null
                    }
                    attemptChannel?.stop()
                    runCatching { attemptConnection?.close() }
                }
            }
        }
    }

    private fun stopStreaming(
        notifyPeer: Boolean,
        keepVideoChannel: Boolean = false,
    ) {
        restartJob?.cancel()
        restartJob = null
        streamingJob?.cancel()
        streamingJob = null
        // The stream is over, so there is no fps or bitrate left to publish.
        // Leaving the tick running only re-posted the same notification once a
        // second, which is what the notification de-duplication below now also
        // guards against.
        stopStatsLoop()
        // Stop Camera2 capture session first so no requests are delivered
        // into a surface that is about to be released by MediaCodec.
        camera.stop()
        encoder?.stop()
        encoder = null
        // A camera switch keeps the channel: the desktop keeps its decoder and
        // the last decoded frame, so the switch has no placeholder gap. Every
        // other teardown closes it, because nothing will send on it again.
        if (!keepVideoChannel) {
            videoChannel?.stop()
            videoChannel = null
            runCatching { video?.close() }
            video = null
            lastStatsBytes.set(0)
        }

        // What the phase becomes after a teardown is a rule, not a local
        // decision: with the control channel open the session falls back to
        // CONNECTED, but a teardown must not overwrite the retry or error state
        // that describes the connection. See [afterStreamStopped].
        _state.value = _state.value.afterStreamStopped(control?.isOpen == true).copy(
            actualResolution = null,
            actualFps = null,
            streamFps = 0.0,
        )

        if (notifyPeer) sendState()
    }

    // ---------------------------------------------------------------- discovery

    private fun phoneBeacon(): JsonObject = buildJsonObject {
        put("t", "phonecam.beacon")
        put("v", Proto.VERSION)
        put("role", "phone")
        put("deviceId", identity.deviceId)
        put("name", identity.deviceName)
        put("model", Build.MODEL)
        put("androidVersion", Build.VERSION.SDK_INT)
        put("appVersion", BuildConfig.VERSION_NAME)
        put("streaming", _state.value.isStreaming)
    }

    fun probeNow() {
        // A search can arrive while one is running — the retry affordance on the
        // error card calls this too — so the previous window is cancelled rather
        // than allowed to report on a search the user has already replaced.
        searchJob?.cancel()
        val previousPhase = _state.value.phase
            .takeUnless { it == SessionState.Phase.DISCOVERING }
            ?: SessionState.Phase.IDLE
        discovery.probe()
        _state.value = _state.value.searchStarted().copy(phase = SessionState.Phase.DISCOVERING)

        searchJob = scope.launch {
            delay(SEARCH_WINDOW_MS)
            _state.value = _state.value
                .searchCompleted()
                .searchPhaseRestored(previousPhase)

            // The outcome is transient by design: "not found" is true of the
            // window that just closed, not of the network forever, and leaving it
            // on screen would read as a permanent verdict on a PC that may have
            // started a moment later.
            delay(SEARCH_RESULT_MS)
            _state.value = _state.value.searchResultCleared()
        }
    }

    /**
     * The user switching this phone's own camera.
     *
     * The desktop is the protocol's driver for `set_camera` (docs/protocol.md
     * §6.2), but the person holding the phone is the one who knows which way it
     * is pointing, so the phone is allowed to choose locally too. The change is
     * not silent: the new camera is reported in the next `state` message, which
     * is how both ends stay in agreement without a new wire message.
     */
    fun selectCameraLocally(facingOrId: String) {
        val target = cameraFor(facingOrId)
        if (target == null) {
            // Not necessarily absent — the capability list may simply not have
            // been probed yet, in which case this is remembered and applied the
            // moment it is (see the init probe).
            if (_state.value.capabilities.isEmpty()) pendingCameraId = facingOrId
            return
        }
        if (target.id == _state.value.selectedCameraId) return

        val wasStreaming = _state.value.isStreaming
        selectCamera(facingOrId)
        Log.i("camera switched on the phone to ${_state.value.selectedCameraId}")
        // A running stream restarts on the new camera and reports the change, and
        // its real resolution, once the camera is up; sending here as well would
        // tell the desktop "not streaming" for the moment in between.
        if (!wasStreaming) sendState()
    }

    /**
     * Re-reads the camera list, keeping the current selection when the camera is
     * still there and falling back to the first one when it is not.
     *
     * [SessionState.activeCamera] is `capabilities.firstOrNull { it.id ==
     * selectedCameraId }`, so refreshing the list without refreshing the id would
     * leave the selection pointing at a camera that is no longer in it — and the
     * UI would show no camera at all despite having just found some. That rule
     * lives in [withCameras] because the startup and service probes need it too.
     */
    fun refreshCapabilities() {
        // The one place the cached probe is deliberately refreshed: the user is
        // asking for a re-read, so the cached result must not answer for it.
        _state.value = _state.value.withCameras(capabilityProbe.probe())
    }

    /** Clears a transient notice once the user has acknowledged it. */
    fun acknowledgeNotice() {
        _state.value = _state.value.copy(notice = null, noticeKind = NoticeKind.Substitution, error = null)
    }

    fun clearError() {
        _state.value = _state.value.copy(error = null)
    }

    /** Short correlator for diagnostics; never the secret itself (PRD §63). */
    fun secretFingerprint(): String? =
        identity.deviceSecretHex?.let { Auth.secretFingerprint(it) }

    private fun kotlinx.serialization.json.JsonPrimitive.contentOrNullSafe(): String? =
        runCatching { content }.getOrNull()

    private fun String.jsonObject(): JsonObject? =
        runCatching { kotlinx.serialization.json.Json.parseToJsonElement(this) as? JsonObject }.getOrNull()

    private companion object {
        const val STATS_INTERVAL_MS = 1000L
        const val VIDEO_RESTART_DELAY_MS = 250L

        /**
         * Stepped waits for a camera that failed to start mid-stream, in order.
         *
         * A wedged HAL needs roughly a second of quiet before it will open a
         * device again; retrying faster than that is what produced the endless
         * open-fail loop. The steps give it that quiet while keeping the first
         * retry fast enough that a switch race is invisible on the desktop.
         */
        val CAMERA_RETRY_DELAYS_MS = longArrayOf(500, 1000, 2000, 4000)
        const val MAX_CAMERA_START_FAILURES = 4

        /**
         * How many times a failed dial is retried before the session gives up.
         *
         * With the 250 ms→8 s backoff this is about a minute of trying: long
         * enough to ride out a desktop app restart or a brief Wi-Fi blip, short
         * enough that a PC that is simply off reaches a visible end state rather
         * than an endless silent retry.
         */
        const val MAX_RECONNECT_ATTEMPTS = 12

        /**
         * How long the phone waits for a human to decide a pairing request.
         *
         * The desktop prompts its user and the decision arrives as a control
         * message, but nothing bounds the prompt: a PC that is never answered
         * would leave the phone waiting forever, which is a real end state, not a
         * slow one. 30 s is long enough to notice a prompt and click it, and
         * short enough that an unattended desktop does not strand the phone; when
         * it expires the attempt is treated as a lost connection and routed into
         * the bounded reconnect path, which either re-dials or ends in the
         * visible "couldn't reach your PC" error.
         */
        const val APPROVAL_TIMEOUT_MS = 30_000L

        /**
         * How long a search stays in [SearchState.Searching].
         *
         * Discovery is two round trips on the local network — one broadcast probe
         * out, one unicast beacon back — and the core also beacons once a second
         * (§3.1), so anything past ~2 s only waits for a reply that is not
         * coming. 4 s leaves room for a single dropped packet and a retry while
         * staying short enough that the user is not left watching a spinner.
         */
        const val SEARCH_WINDOW_MS = 4000L

        /** How long a finished outcome stays on screen before fading to Idle. */
        const val SEARCH_RESULT_MS = 4000L
    }
}
