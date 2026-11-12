package com.phonecam.camera.service

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.view.Surface
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.phonecam.camera.R
import com.phonecam.camera.camera.CameraCapabilityProbe
import com.phonecam.camera.data.IdentityStore
import com.phonecam.camera.protocol.CameraInfo
import com.phonecam.camera.protocol.Log
import com.phonecam.camera.session.SessionIntents
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.session.StreamSession
import com.phonecam.camera.session.withCameras
import com.phonecam.camera.ui.MainActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Owns the one live [StreamSession] and keeps it alive while the app is not.
 *
 * The session is a foreground service for two reasons that both come from the
 * product, not from convenience:
 *
 *  - the camera must keep working when the user switches to another app, which is
 *    the normal case for a webcam (PRD §29);
 *  - Android 14 and later refuse to open the camera from the background at all,
 *    so a camera-typed foreground service is the only legal way to stream while
 *    the activity is not on screen.
 *
 * The service is a thin shell: it translates intents into [StreamSession] calls
 * and mirrors the session state into a notification and into [sessionState],
 * which the UI collects. It never talks to the camera or the network itself.
 */
class CameraStreamService : Service() {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val lock = Any()

    private var session: StreamSession? = null
    private var stateJob: Job? = null
    private var foregroundStarted = false

    /**
     * What the currently posted notification displays.
     *
     * The notification shows only the phase and, for some phases, the server
     * name and the error. Everything else in [SessionState] — fps, battery,
     * thermal — changes every second and would otherwise rebuild the
     * notification (two `PendingIntent`s and several strings) just as often.
     */
    private var lastNotificationSignature: NotificationSignature? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        liveInstance = this
        createChannel()
        // A recreated process must not inherit a stale "streaming" claim from a
        // previous incarnation of the service.
        publish(idleState())
        setRunning(false)
        // The phone's cameras are knowable before any desktop is, and the UI
        // shows them while the session is idle — that is the difference between
        // naming the camera in use and saying "Unknown" on a cold start. It runs
        // off the main thread because the probe touches the camera and codec
        // services, the same reason the view model keeps its encoder query off it.
        scope.launch {
            val cameras = withContext(Dispatchers.IO) {
                // The one camera probe for the process, shared with the view
                // model and the session. Probing four times on startup was four
                // walks of CameraManager and MediaCodecList for one answer.
                runCatching { CameraCapabilityProbe.cameras(applicationContext) }
                    .getOrDefault(emptyList())
            }
            if (cameras.isEmpty()) return@launch
            // A session opened while the probe was running reports its own,
            // fresher list; this seed only fills the gap before one exists.
            if (!synchronized(lock) { session == null }) return@launch
            probedCameras = cameras
            // Merged rather than replaced: the user may already have been shown a
            // permission error, and the camera list must not clear it.
            publish(_sessionState.value.withCameras(cameras))
        }
        Log.i("stream service created")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // A null intent means the platform restarted us after a kill, not that
        // the user asked for anything. Re-opening a session here would turn the
        // camera and the microphone-less broadcast back on with nobody looking
        // at the screen, which is exactly what a camera app must not do. The
        // service stops itself; the user starts it again from the app.
        if (intent == null) {
            Log.i("stream service restarted with no intent; not reopening the session")
            stopSelf()
            return START_NOT_STICKY
        }

        val action = intent.action ?: SessionIntents.ACTION_START
        val stopReason = intent.getStringExtra(SessionIntents.EXTRA_STOP_REASON)

        if (action == SessionIntents.ACTION_STOP && stopReason == SessionIntents.STOP_REASON_USER) {
            // A stop still has to satisfy the foreground contract if the platform
            // started us as a foreground service, so claim the slot first.
            startForegroundNow()
            endSession()
            return START_NOT_STICKY
        }

        if (!startForegroundNow()) {
            // No camera permission means no camera-typed foreground service and no
            // camera. Report it as state the UI can explain; never crash (Rule 6).
            Log.w("session not started: camera permission is not granted")
            publish(
                idleState().copy(
                    phase = SessionState.Phase.ERROR,
                    error = getString(R.string.permission_denied_body),
                ),
            )
            setRunning(false)
            stopSelf()
            return START_NOT_STICKY
        }

        val active = session ?: openSession()

        when (action) {
            SessionIntents.ACTION_START -> Log.i("session start requested")

            SessionIntents.ACTION_SEARCH -> active.probeNow()

            SessionIntents.ACTION_DISCONNECT -> active.disconnect()

            SessionIntents.ACTION_REFRESH_CAPABILITIES -> active.refreshCapabilities()

            SessionIntents.ACTION_SELECT_CAMERA -> {
                val camera = intent?.getStringExtra(SessionIntents.EXTRA_CAMERA)
                if (camera.isNullOrBlank()) {
                    Log.w("camera select without a camera")
                } else {
                    active.selectCameraLocally(camera)
                }
            }

            SessionIntents.ACTION_ACKNOWLEDGE_NOTICE -> active.acknowledgeNotice()

            SessionIntents.ACTION_SET_PREVIEW -> {
                if (intent == null || !intent.hasExtra(SessionIntents.EXTRA_PREVIEW_ENABLED)) {
                    Log.w("preview change without a value")
                } else {
                    active.setPreviewEnabled(
                        intent.getBooleanExtra(SessionIntents.EXTRA_PREVIEW_ENABLED, true),
                    )
                }
            }

            SessionIntents.ACTION_CONNECT_DISCOVERED -> {
                val host = intent?.getStringExtra(SessionIntents.EXTRA_HOST)
                val port = intent?.getIntExtra(SessionIntents.EXTRA_PORT, SessionIntents.DEFAULT_PORT)
                    ?: SessionIntents.DEFAULT_PORT
                val serverId = intent?.getStringExtra(SessionIntents.EXTRA_SERVER_ID)
                val discovered = active.state.value.discovered
                    .firstOrNull { it.serverId == serverId && it.host == host }
                when {
                    discovered != null -> active.connectTo(discovered)
                    host != null -> active.connectManually(host, port)
                    else -> Log.w("connect request without a host")
                }
            }

            SessionIntents.ACTION_CONNECT_MANUAL -> {
                val host = intent?.getStringExtra(SessionIntents.EXTRA_HOST)
                val port = intent?.getIntExtra(SessionIntents.EXTRA_PORT, SessionIntents.DEFAULT_PORT)
                    ?: SessionIntents.DEFAULT_PORT
                if (host.isNullOrBlank()) {
                    Log.w("manual connect without a host")
                } else {
                    active.connectManually(host, port)
                }
            }

            SessionIntents.ACTION_CONNECT_TOKEN -> {
                val host = intent?.getStringExtra(SessionIntents.EXTRA_HOST)
                val port = intent?.getIntExtra(SessionIntents.EXTRA_PORT, SessionIntents.DEFAULT_PORT)
                    ?: SessionIntents.DEFAULT_PORT
                val token = intent?.getStringExtra(SessionIntents.EXTRA_TOKEN)
                if (host.isNullOrBlank() || token.isNullOrBlank()) {
                    Log.w("pairing connect without a host or token")
                } else {
                    active.connectWithToken(host, port, token)
                }
            }

            else -> Log.d("ignoring unknown session action $action")
        }

        // Never sticky. The session is a user-initiated, camera-owning activity;
        // a service the platform killed must not be resurrected to re-open the
        // camera and resume broadcasting with no one watching. The user starts it
        // again from the app, where the state is visible. A process-level kill
        // still lets the desktop reconnect once the phone is restarted.
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        if (liveInstance === this) liveInstance = null
        stateJob?.cancel()
        stateJob = null
        synchronized(lock) {
            session?.stop()
            session = null
        }
        setRunning(false)
        scope.cancel()
        Log.i("stream service destroyed")
        super.onDestroy()
    }

    // ------------------------------------------------------------------ session

    private fun openSession(): StreamSession = synchronized(lock) {
        session ?: StreamSession(applicationContext, IdentityStore(applicationContext)).also { created ->
            session = created
            // The session probes its own cameras asynchronously, so its first
            // state has none yet; merging in what the service already found keeps
            // the UI from blinking back to "Unknown" while that probe runs.
            publish(created.state.value.withCameras(probedCameras))
            setRunning(true)
            stateJob = scope.launch {
                created.state.collect { state ->
                    publish(state)
                    updateNotification(state)
                }
            }
            created.start()
            Log.i("session opened")
        }
    }

    private fun endSession() {
        stateJob?.cancel()
        stateJob = null
        synchronized(lock) {
            session?.stop()
            session = null
        }
        foregroundStarted = false
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        publish(idleState())
        setRunning(false)
        stopSelf()
        Log.i("session ended by the user")
    }

    // ------------------------------------------------------------ notification

    /**
     * Claims the foreground slot, and says whether the camera type was allowed.
     *
     * The platform's contract is unconditional: once `startForegroundService` has
     * been called, `startForeground` must follow within five seconds or the
     * process is killed with `ForegroundServiceDidNotStartInTimeException`. This
     * function used to return `false` *without* calling it when the camera
     * permission was missing, so the one path a first-time user always takes —
     * start the app, accept nothing yet — crashed the app instead of explaining
     * what was missing. The permission decides whether the service may be typed
     * `camera`; it never decides whether the slot is claimed.
     *
     * The caller gets `false` and is expected to stop the service immediately,
     * which it does: the notification is posted, the service is torn down, and
     * the UI shows the permission card.
     */
    private fun startForegroundNow(): Boolean {
        val cameraGranted = ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
        if (foregroundStarted) return cameraGranted

        val state = sessionState.value
        val built = buildNotification(state)
        lastNotificationSignature = signatureOf(state)
        // A camera-typed foreground service requires the camera permission. Without
        // it the slot is still claimed, untyped, so the contract is satisfied and
        // the service can shut itself down cleanly.
        val type = if (cameraGranted && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        } else {
            0
        }
        return runCatching {
            ServiceCompat.startForeground(this, NOTIFICATION_ID, built, type)
            foregroundStarted = true
            cameraGranted
        }.onFailure { error ->
            Log.w("could not become a foreground service: ${error.javaClass.simpleName}")
        }.getOrDefault(false)
    }

    private fun updateNotification(state: SessionState) {
        if (!foregroundStarted) return
        // Rebuild only when something the notification actually shows changed.
        // The once-per-second stats tick copies the state on every frame, and
        // rebuilding then allocated two PendingIntents and several strings for a
        // notification whose text was identical.
        val signature = signatureOf(state)
        if (signature == lastNotificationSignature) return

        val built = buildNotification(state)
        val allowed = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        if (!allowed) return
        runCatching {
            NotificationManagerCompat.from(this).notify(NOTIFICATION_ID, built)
            lastNotificationSignature = signature
        }.onFailure { Log.d("notification update failed: ${it.javaClass.simpleName}") }
    }

    /** The subset of [SessionState] that the notification's text depends on. */
    private fun signatureOf(state: SessionState): NotificationSignature = NotificationSignature(
        phase = state.phase,
        serverName = state.serverName,
        error = state.error,
    )

    private fun buildNotification(state: SessionState): Notification {
        val title: String
        val text: String
        when (state.phase) {
            SessionState.Phase.IDLE -> {
                title = getString(R.string.notification_title_ready)
                text = getString(R.string.notification_text_idle)
            }

            SessionState.Phase.DISCOVERING,
            SessionState.Phase.CONNECTING,
            SessionState.Phase.RECONNECTING,
            -> {
                title = getString(R.string.notification_title_connecting)
                text = state.serverName?.let { getString(R.string.notification_text_connecting_to, it) }
                    ?: getString(R.string.notification_text_connecting)
            }

            SessionState.Phase.CONNECTED -> {
                title = getString(R.string.notification_title_ready)
                text = state.serverName?.let { getString(R.string.notification_text_connected, it) }
                    ?: getString(R.string.notification_text_idle)
            }

            SessionState.Phase.STREAMING -> {
                title = getString(R.string.notification_title_streaming)
                text = state.serverName?.let { getString(R.string.notification_text_streaming, it) }
                    ?: getString(R.string.notification_text_streaming_unknown)
            }

            SessionState.Phase.ERROR -> {
                title = getString(R.string.notification_title_error)
                text = state.error ?: getString(R.string.notification_text_idle)
            }
        }

        val contentIntent = PendingIntent.getActivity(
            this,
            REQUEST_OPEN,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val stopIntent = PendingIntent.getService(
            this,
            REQUEST_STOP,
            SessionIntents.stop(this),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_stat_phonecam)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setContentIntent(contentIntent)
            .addAction(0, getString(R.string.notification_action_stop), stopIntent)
            .setOngoing(true)
            .setSilent(true)
            .setShowWhen(false)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(NotificationManager::class.java) ?: return
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.notification_channel_name),
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = getString(R.string.notification_channel_description)
            setShowBadge(false)
            enableVibration(false)
        }
        manager.createNotificationChannel(channel)
    }

    private fun publish(state: SessionState) {
        _sessionState.value = state
    }

    /**
     * The state of a phone with no session: no stream, no desktop, but its own
     * cameras already known.
     *
     * Everything else in [SessionState] defaults to "nothing to report", which is
     * exactly right before a session exists; the cameras are the one fact the
     * phone can supply on its own, and without them the UI would call the camera
     * in use "Unknown" until the first `capabilities` message.
     */
    private fun idleState(): SessionState {
        val cameras = probedCameras
        return SessionState(
            capabilities = cameras,
            selectedCameraId = cameras.firstOrNull()?.id,
        )
    }

    private fun setRunning(running: Boolean) {
        _sessionRunning.value = running
    }

    /**
     * Hands the live session a viewfinder surface, or `null` to withdraw it.
     *
     * A `Surface` cannot travel through the `Intent` the UI otherwise uses to
     * talk to the service, and the viewfinder is a display concern the session
     * must not own. This is the one direct call across that boundary; it is safe
     * because both live in the same process and the service holds the only
     * session.
     */
    private fun setPreviewSurface(surface: Surface?) {
        session?.setPreviewSurface(surface)
    }

    /**
     * The fields [buildNotification] actually reads, used to skip rebuilds that
     * would produce an identical notification.
     */
    private data class NotificationSignature(
        val phase: SessionState.Phase,
        val serverName: String?,
        val error: String?,
    )

    companion object {
        private const val CHANNEL_ID = "phonecam.session"
        private const val NOTIFICATION_ID = 4201
        private const val REQUEST_OPEN = 10
        private const val REQUEST_STOP = 11

        private val _sessionState = MutableStateFlow(SessionState())

        /** The live session state; the UI renders this and nothing else. */
        val sessionState: StateFlow<SessionState> = _sessionState.asStateFlow()

        /**
         * The running service, so the UI can hand it a viewfinder surface.
         *
         * Held statically for the same reason the state flows are: there is one
         * service and one session in this process, and the activity has no binding
         * to it. Cleared on destroy so a stale instance is never called.
         */
        @Volatile
        private var liveInstance: CameraStreamService? = null

        /** Forwards a viewfinder surface to the live session, if there is one. */
        fun previewSurface(surface: Surface?) {
            liveInstance?.setPreviewSurface(surface)
        }

        /**
         * The cameras the service probed for itself, kept across instances.
         *
         * Static because they describe the phone rather than a session: the list
         * stays true after the session ends and after the service is destroyed
         * and recreated, so a restart does not blink back to "Unknown" while the
         * probe runs again.
         */
        private var probedCameras: List<CameraInfo> = emptyList()

        private val _sessionRunning = MutableStateFlow(false)

        /** Whether a session exists at all, which is what the start/stop button reflects. */
        val sessionRunning: StateFlow<Boolean> = _sessionRunning.asStateFlow()
    }
}
