package com.phonecam.camera.ui

import android.Manifest
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.os.Bundle
import android.view.Surface
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.BatteryChargingFull
import androidx.compose.material.icons.filled.BatteryFull
import androidx.compose.material.icons.filled.MonitorHeart
import androidx.compose.material.icons.filled.Output
import androidx.compose.material.icons.filled.PhotoCamera
import androidx.compose.material.icons.filled.Troubleshoot
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.delay
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.viewmodel.compose.viewModel
import com.phonecam.camera.R
import com.phonecam.camera.protocol.BugReport
import com.phonecam.camera.protocol.Log
import com.phonecam.camera.protocol.ReportDevice
import com.phonecam.camera.protocol.ReportSession
import com.phonecam.camera.session.NoticeKind
import com.phonecam.camera.session.SessionState
import com.phonecam.camera.session.connectedTransport
import com.phonecam.camera.transport.DiscoveredServer
import com.phonecam.camera.ui.components.CameraControlCard
import com.phonecam.camera.ui.components.CameraPermissionCard
import com.phonecam.camera.ui.components.CameraPreview
import com.phonecam.camera.ui.components.CameraCapabilitySection
import com.phonecam.camera.ui.components.ConnectionCard
import com.phonecam.camera.ui.components.DiagnosticsCard
import com.phonecam.camera.ui.components.ErrorCard
import com.phonecam.camera.ui.components.Hairline
import com.phonecam.camera.ui.components.HealthCard
import com.phonecam.camera.ui.components.Hint
import com.phonecam.camera.ui.components.SectionCard
import com.phonecam.camera.ui.components.InfoBanner
import com.phonecam.camera.ui.components.NotificationPermissionNote
import com.phonecam.camera.ui.components.OutputCard
import com.phonecam.camera.ui.components.PermissionDeniedCard
import com.phonecam.camera.ui.components.PhasePill
import com.phonecam.camera.ui.components.StatusCard
import com.phonecam.camera.ui.components.StreamingOverlay
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.PhoneCamTheme

/**
 * PhoneCam's control panel, as four screens behind a bottom bar.
 *
 * It is a control panel, not a viewfinder: the phone is usually pointed at
 * something other than the user, so the screen shows the state of the session and
 * the things that need a decision. The frames go to the encoder and to the PC, so
 * the only preview is the deliberate one on the Status screen (PRD §46).
 *
 * The four screens are the four questions a user has, in the order they ask them:
 *
 *  - **Status** — is the phone connected, and is a picture flowing? Everything
 *    that blocks the stream (permissions, errors, a substitution) lives here
 *    too, because a blocked stream is the loudest possible answer to "is it
 *    working".
 *  - **Camera** — what this phone captures: which camera, and what that camera
 *    can do.
 *  - **Output** — what the desktop receives: the stream's shape ([StatusCard])
 *    and the output settings it runs with.
 *  - **Diagnostics** — numbers and the log.
 *
 * The phase and the one start/stop action sit in a strip above the bar, so the
 * answer to "is it working" and the way to end a stream are reachable from every
 * screen. [StreamingOverlay] and the blackout surface stay outside this shell:
 * they are full-window states, not destinations.
 *
 * The activity owns no session state. [PhoneCamViewModel] forwards to the
 * foreground service, and everything drawn comes from the service's
 * [SessionState], which is what makes rotation, backgrounding, and "the user
 * pressed Home" all harmless. The selected destination is the only thing held
 * here, in [rememberSaveable], so a rotation lands on the same screen.
 */
class MainActivity : ComponentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        Log.i("main activity created")
        setContent {
            PhoneCamTheme {
                PhoneCamScreen(viewModel = viewModel())
            }
        }
    }
}

/**
 * The four destinations.
 *
 * The icons come from `material-icons-extended`, which is the dependency this
 * module actually declares; the core artifact that ships with Material3 does not
 * carry all four.
 */
private enum class ScreenDestination(
    val labelRes: Int,
    val icon: ImageVector,
) {
    Status(R.string.nav_status, Icons.Filled.MonitorHeart),
    Camera(R.string.nav_camera, Icons.Filled.PhotoCamera),
    Output(R.string.nav_output, Icons.Filled.Output),
    Diagnostics(R.string.nav_diagnostics, Icons.Filled.Troubleshoot),
}

@Composable
private fun PhoneCamScreen(viewModel: PhoneCamViewModel) {
    val context = LocalContext.current
    val state by viewModel.state.collectAsState()
    val isRunning by viewModel.sessionRunning.collectAsState()
    val notice by viewModel.notice.collectAsState()
    val noticeKind by viewModel.noticeKind.collectAsState()
    val identity by viewModel.identity.collectAsState()
    val logLines by viewModel.logLines.collectAsState()
    val toast by viewModel.toast.collectAsState()
    val hasHardwareEncoder by viewModel.hasHardwareEncoder.collectAsState()
    val camerasProbed by viewModel.camerasProbed.collectAsState()
    val notificationNoteDismissed by viewModel.notificationNoteDismissed.collectAsState()
    val previewEnabled by viewModel.previewEnabled.collectAsState()

    // The desktop's `set_screen` choice or local phone blackout, applied to this window.
    // Active only while streaming, so opening the app never pins the screen on.
    val screenMode = ScreenMode.fromWire(state.screenMode)
    var localBlackoutActive by rememberSaveable { mutableStateOf(false) }

    LaunchedEffect(state.phase) {
        if (state.phase != SessionState.Phase.STREAMING) {
            localBlackoutActive = false
        }
    }

    val isStreaming = state.phase == SessionState.Phase.STREAMING
    val isBlackoutMode = (screenMode == ScreenMode.BLACK || localBlackoutActive)
    ApplyScreenMode(
        mode = if (localBlackoutActive) ScreenMode.BLACK else screenMode,
        active = isStreaming,
    )

    // In `black` mode, a tap uncovers the UI until the mode changes again.
    var blackoutPeeked by remember(screenMode, localBlackoutActive) { mutableStateOf(false) }

    // The selected destination survives rotation. Held as an ordinal because the
    // saveable registry takes primitives, not enums.
    var destinationOrdinal by rememberSaveable { mutableIntStateOf(0) }
    val destination = ScreenDestination.entries[destinationOrdinal]

    var cameraGranted by remember { mutableStateOf(context.hasCameraPermission()) }
    var notificationGranted by remember { mutableStateOf(context.hasNotificationPermission()) }
    var permissionRequested by rememberSaveable { mutableStateOf(false) }

    val snackbarHost = remember { SnackbarHostState() }

    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { results ->
        cameraGranted = results[Manifest.permission.CAMERA] == true || context.hasCameraPermission()
        notificationGranted = context.hasNotificationPermission()
        permissionRequested = true
        if (cameraGranted) {
            // The user asked for a stream and the permission was the only thing
            // in the way, so start it now instead of making them ask twice.
            viewModel.startSession()
        }
    }

    val scanLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        // A code that did not parse. This is the case the "isn't a PhoneCam
        // pairing code" message is actually true for; a plain cancel (the user
        // backed out) stays silent.
        if (result.resultCode == QrScanActivity.RESULT_REJECTED) {
            viewModel.reportInvalidQr()
            return@rememberLauncherForActivityResult
        }
        if (result.resultCode != android.app.Activity.RESULT_OK) return@rememberLauncherForActivityResult
        val data = result.data
        val host = data?.getStringExtra(QrScanActivity.EXTRA_HOST)
        val port = data?.getIntExtra(QrScanActivity.EXTRA_PORT, 0) ?: 0
        val token = data?.getStringExtra(QrScanActivity.EXTRA_TOKEN)
        if (host.isNullOrBlank() || token.isNullOrBlank() || port !in 1..65535) {
            viewModel.reportInvalidQr()
        } else {
            viewModel.connectWithToken(host, port, token)
            // The code parsed and the dial is starting. The name in the code is
            // optional and the *outcome* is what establishes the PC's real name,
            // so the pairing is announced from the connected state (see
            // [PhoneCamViewModel.announceChange]); this only tells the user the
            // scan was accepted, so scanning a code is never a silent no-op.
            viewModel.reportQrDispatched()
        }
    }

    // The camera permission can also change while the app is in the background
    // (the user flipping the switch in Settings), so re-read it on every resume.
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                cameraGranted = context.hasCameraPermission()
                notificationGranted = context.hasNotificationPermission()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    LaunchedEffect(toast) {
        val message = toast ?: return@LaunchedEffect
        snackbarHost.showSnackbar(message)
        viewModel.consumeToast()
    }

    val requestCamera: () -> Unit = {
        permissionRequested = true
        permissionLauncher.launch(context.permissionsToRequest())
    }
    val scanQr: () -> Unit = {
        scanLauncher.launch(Intent(context, QrScanActivity::class.java))
    }
    val startOrRequestCamera: () -> Unit = {
        if (cameraGranted) viewModel.startSession() else requestCamera()
    }

    val encoderLabel = stringResource(
        when (hasHardwareEncoder) {
            true -> R.string.diagnostics_encoder_hardware
            false -> R.string.diagnostics_encoder_software
            // The probe has not answered yet; saying "Software" would assert an
            // encoder that may not exist.
            null -> R.string.diagnostics_not_detected
        },
    )

    // The report is built on demand from the live state, so it describes the
    // session as it is at the moment the button is pressed rather than at
    // composition. It is assembled here because the card renders only the values
    // it is handed.
    val buildReport: () -> String = {
        BugReport.build(
            appVersion = viewModel.appVersion,
            deviceId = identity.deviceId,
            deviceName = identity.deviceName,
            device = ReportDevice.current(),
            session = ReportSession(
                phase = state.phase.name,
                // The honest transport: `state.transport` is the link a dial is
                // *using* and defaults to "lan", so the report would claim a LAN
                // link on a phone that has never connected. `connectedTransport()`
                // is null outside CONNECTED/STREAMING, and the builder prints
                // "Not connected" for that.
                transport = state.connectedTransport(),
                camera = state.activeCamera?.id ?: "",
                requestedResolution = state.requestedResolution,
                requestedFps = state.requestedFps?.toString() ?: "",
                actualResolution = state.actualResolution ?: "",
                actualFps = state.actualFps?.toString() ?: "",
                encoder = encoderLabel,
                batteryPercent = state.batteryPercent,
                charging = state.charging,
                thermal = state.thermal,
            ),
        )
    }

    var pendingCrashReport by remember { mutableStateOf(com.phonecam.camera.protocol.CrashReporter.readPendingCrashReport(context)) }
    if (pendingCrashReport != null) {
        val report = pendingCrashReport!!
        AlertDialog(
            onDismissRequest = {
                com.phonecam.camera.protocol.CrashReporter.clearPendingCrashReport(context)
                pendingCrashReport = null
            },
            title = { Text("Session Crash Detected") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        "PhoneCam stopped unexpectedly in a previous session. You can report this to GitHub to help us fix it quickly.",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    Text(
                        report.take(300) + if (report.length > 300) "..." else "",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            },
            confirmButton = {
                Button(
                    onClick = {
                        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        clipboard.setPrimaryClip(ClipData.newPlainText("Crash Report", report))
                        val intent = com.phonecam.camera.protocol.CrashReporter.buildGitHubIssueIntent(
                            context = context,
                            title = "[Crash] Android App Crash Report",
                            bodyMarkdown = "### Crash Report\n```\n$report\n```",
                        )
                        runCatching { context.startActivity(intent) }
                        com.phonecam.camera.protocol.CrashReporter.clearPendingCrashReport(context)
                        pendingCrashReport = null
                    }
                ) {
                    Text("Report to GitHub")
                }
            },
            dismissButton = {
                TextButton(
                    onClick = {
                        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        clipboard.setPrimaryClip(ClipData.newPlainText("Crash Report", report))
                        com.phonecam.camera.protocol.CrashReporter.clearPendingCrashReport(context)
                        pendingCrashReport = null
                    }
                ) {
                    Text("Copy & Dismiss")
                }
            }
        )
    }

    Box(modifier = Modifier.fillMaxSize()) {
        Scaffold(
            snackbarHost = { SnackbarHost(snackbarHost) },
            containerColor = MaterialTheme.colorScheme.background,
            bottomBar = {
                Column {
                    // The one thing that must be reachable from everywhere: is it
                    // working, and how do I stop it.
                    StatusStrip(
                        state = state,
                        isRunning = isRunning,
                        onStart = startOrRequestCamera,
                        onStop = { viewModel.stopSession() },
                    )
                    NavigationBar {
                        ScreenDestination.entries.forEachIndexed { index, item ->
                            NavigationBarItem(
                                selected = item == destination,
                                onClick = { destinationOrdinal = index },
                                icon = {
                                    Icon(
                                        imageVector = item.icon,
                                        contentDescription = null,
                                    )
                                },
                                label = { Text(stringResource(item.labelRes)) },
                            )
                        }
                    }
                }
            },
        ) { insets ->
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(insets),
            ) {
                when (destination) {
                    ScreenDestination.Status -> StatusScreen(
                        state = state,
                        isRunning = isRunning,
                        cameraGranted = cameraGranted,
                        notificationGranted = notificationGranted,
                        notificationNoteDismissed = notificationNoteDismissed,
                        permissionRequested = permissionRequested,
                        notice = notice,
                        noticeKind = noticeKind,
                        pairedServerName = identity.pairedServerName,
                        pairedAddress = identity.pairedAddress,
                        buildReport = buildReport,
                        onRequestCamera = requestCamera,
                        onDismissNotificationNote = { viewModel.dismissNotificationNote() },
                        onRetry = { viewModel.retry() },
                        onScanQr = scanQr,
                        onSearch = { viewModel.searchNow() },
                        onConnectDiscovered = { server -> viewModel.connectTo(server) },
                        onConnectManual = { host, port -> viewModel.connectManually(host, port) },
                        onForget = { viewModel.forgetPairing() },
                        onSurfaceAvailable = { surface -> viewModel.setPreviewSurface(surface) },
                        onAcknowledgeNotice = { viewModel.acknowledgeNotice() },
                        previewEnabled = previewEnabled,
                        onPreviewEnabled = { enabled -> viewModel.setPreviewEnabled(enabled) },
                        onEnableBlackout = {
                            localBlackoutActive = true
                            blackoutPeeked = false
                        },
                    )

                    ScreenDestination.Camera -> CameraScreen(
                        state = state,
                        onSelectCamera = { cameraId -> viewModel.selectCamera(cameraId) },
                    )

                    ScreenDestination.Output -> OutputScreen(
                        state = state,
                        previewEnabled = previewEnabled,
                        onPreviewEnabled = { enabled -> viewModel.setPreviewEnabled(enabled) },
                        onEnableBlackout = {
                            localBlackoutActive = true
                            blackoutPeeked = false
                        },
                    )

                    ScreenDestination.Diagnostics -> DiagnosticsScreen(
                        state = state,
                        identity = identity,
                        encoderLabel = encoderLabel,
                        camerasProbed = camerasProbed,
                        appVersion = viewModel.appVersion,
                        logLines = logLines,
                        buildReport = buildReport,
                        onRefreshCapabilities = { viewModel.refreshCapabilities() },
                        onClearLog = { viewModel.clearLog() },
                    )
                }
            }
        }

        if (isStreaming && isBlackoutMode && blackoutPeeked) {
            // When peeked, tapping outside the controls returns back to black ambient mode
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                        onClick = { blackoutPeeked = false },
                    ),
            )
        }

        if (isStreaming) {
            // What the user sees when the phone is face-down next to the
            // keyboard: proof the stream is alive, and a way to end it.
            StreamingOverlay(
                state = state,
                onStop = { viewModel.stopSession() },
                modifier = Modifier.align(Alignment.BottomCenter),
            )
        }

        if (isStreaming && isBlackoutMode && !blackoutPeeked) {
            // OLED Pure-Black Ambient Mode: true-black #000000 with minimal dim clock,
            // anti-burn-in offset, battery, and live badge. Tapping reveals controls.
            BlackoutSurface(state = state, onTap = { blackoutPeeked = true })
        }
    }
}

/**
 * The compact status strip above the navigation bar.
 *
 * It is the answer to "is it working" and the way to end a stream, and it is the
 * one part of the shell that never scrolls away or changes with the destination.
 * It folds in what [LiveStatusHeader] and the start/stop button in
 * [ConnectionCard] used to say — the phase and the single action — without the
 * long sentence, because the details belong on the Status screen it points at.
 */
@Composable
private fun StatusStrip(
    state: SessionState,
    isRunning: Boolean,
    onStart: () -> Unit,
    onStop: () -> Unit,
    modifier: Modifier = Modifier,
) {
    // A short line beside the pill, or nothing: the pill already names the phase,
    // so this only adds the one fact the pill cannot carry — where the stream is
    // going.
    val detail: String? = when (state.phase) {
        SessionState.Phase.STREAMING ->
            state.serverName
                ?.let { stringResource(R.string.overlay_streaming_title, it) }
                ?: stringResource(R.string.overlay_streaming_title_unknown)

        SessionState.Phase.CONNECTED ->
            state.serverName?.let { stringResource(R.string.notification_text_connected, it) }

        SessionState.Phase.DISCOVERING,
        SessionState.Phase.CONNECTING,
        -> stringResource(R.string.notification_text_connecting)

        else -> null
    }

    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceContainer),
    ) {
        Hairline()
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            PhasePill(state = state)
            Text(
                text = detail.orEmpty(),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Button(
                onClick = if (isRunning) onStop else onStart,
                contentPadding = PaddingValues(horizontal = 20.dp, vertical = 6.dp),
            ) {
                Text(
                    stringResource(
                        if (isRunning) R.string.action_stop_session else R.string.action_start_session,
                    ),
                )
            }
        }
    }
}

/** The scrolling body every screen shares, so the gaps and margins cannot drift. */
@Composable
private fun ScreenScroll(
    modifier: Modifier = Modifier,
    horizontalPadding: androidx.compose.ui.unit.Dp = 16.dp,
    content: @Composable ColumnScope.() -> Unit,
) {
    val isLandscape = LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.TopCenter,
    ) {
        Column(
            modifier = modifier
                .fillMaxSize()
                .then(if (isLandscape && horizontalPadding == 0.dp) Modifier.fillMaxWidth() else Modifier.widthIn(max = 720.dp))
                .verticalScroll(rememberScrollState())
                .padding(
                    PaddingValues(
                        horizontal = horizontalPadding,
                        vertical = if (isLandscape) 8.dp else 16.dp,
                    ),
                ),
            // The gaps carry the grouping. A card, a banner and a section are three
            // different weights of thing, so they get three different gaps instead of
            // one uniform stack of boxes.
            verticalArrangement = Arrangement.spacedBy(16.dp),
            content = content,
        )
    }
}

/**
 * Is the phone connected and is a picture flowing.
 *
 * The blockers come first, in the order they block: no permission, no session, a
 * failure, a substitution. The connection controls and the connection facts
 * follow, and the preview sits among them because it is the most direct answer to
 * the screen's question.
 */
@Composable
private fun StatusScreen(
    state: SessionState,
    isRunning: Boolean,
    cameraGranted: Boolean,
    notificationGranted: Boolean,
    notificationNoteDismissed: Boolean,
    permissionRequested: Boolean,
    notice: String?,
    noticeKind: NoticeKind,
    pairedServerName: String?,
    pairedAddress: String?,
    buildReport: () -> String,
    onRequestCamera: () -> Unit,
    onDismissNotificationNote: () -> Unit,
    onRetry: () -> Unit,
    onScanQr: () -> Unit,
    onSearch: () -> Unit,
    onConnectDiscovered: (DiscoveredServer) -> Unit,
    onConnectManual: (String, Int) -> Unit,
    onForget: () -> Unit,
    onSurfaceAvailable: (Surface?) -> Unit,
    onAcknowledgeNotice: () -> Unit,
    previewEnabled: Boolean,
    onPreviewEnabled: (Boolean) -> Unit,
    onEnableBlackout: () -> Unit,
) {
    val context = LocalContext.current
    val isLandscape = LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    ScreenScroll(horizontalPadding = if (isLandscape) 0.dp else 16.dp) {
        // The things that block everything else.
        if (!cameraGranted) {
            Box(modifier = if (isLandscape) Modifier.padding(horizontal = 16.dp) else Modifier) {
                // A permanent denial gets the Settings route, because the platform
                // will no longer show a dialog (PRD §39: explain, do not nag).
                if (permissionRequested && !context.shouldShowCameraRationale()) {
                    PermissionDeniedCard(onOpenSettings = { context.openAppSettings() })
                } else {
                    CameraPermissionCard(onRequest = onRequestCamera)
                }
            }
        } else if (!notificationGranted && !notificationNoteDismissed) {
            Box(modifier = if (isLandscape) Modifier.padding(horizontal = 16.dp) else Modifier) {
                NotificationPermissionNote(onDismiss = onDismissNotificationNote)
            }
        }

        // A failure is calm prose with a way out, never a wall (PRD §38).
        if (state.phase == SessionState.Phase.ERROR && state.error != null) {
            Box(modifier = if (isLandscape) Modifier.padding(horizontal = 16.dp) else Modifier) {
                ErrorCard(
                    message = state.error,
                    onRetry = onRetry,
                    secondaryLabel = stringResource(R.string.action_scan_qr),
                    onSecondary = onScanQr,
                    onReportIssue = {
                        val reportText = buildReport()
                        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        clipboard.setPrimaryClip(ClipData.newPlainText("Diagnostics Report", reportText))
                        val intent = com.phonecam.camera.protocol.CrashReporter.buildGitHubIssueIntent(
                            context = context,
                            title = "[Error] Stream error: ${state.error}",
                            bodyMarkdown = "### Error Message\n${state.error}\n\n### System Diagnostics\n```\n$reportText\n```",
                        )
                        runCatching { context.startActivity(intent) }
                    },
                )
            }
        } else if (state.phase == SessionState.Phase.CONNECTING || state.phase == SessionState.Phase.RECONNECTING) {
            Box(modifier = if (isLandscape) Modifier.padding(horizontal = 16.dp) else Modifier) {
                ConnectingCard()
            }
        }

        // A capability substitution is normal operation, so it is an inline
        // banner rather than a dialog (PRD §13). An approval instruction is a
        // notice too but not a substitution, so it gets an accurate heading while
        // staying in the same blue container.
        notice?.let { text ->
            Box(modifier = if (isLandscape) Modifier.padding(horizontal = 16.dp) else Modifier) {
                val title = when (noticeKind) {
                    NoticeKind.Approval -> stringResource(R.string.banner_approval_title)
                    NoticeKind.Substitution -> stringResource(R.string.banner_notice_title)
                }
                InfoBanner(text = text, title = title, onDismiss = onAcknowledgeNotice)
            }
        }

        // Advice about the phone itself. Renders nothing when there is none, so
        // it costs no space in the common case.
        Box(modifier = if (isLandscape) Modifier.padding(horizontal = 16.dp) else Modifier) {
            HealthCard(state = state)
        }

        // The picture itself, once there is one. Idle, it is absent rather than
        // blank: a black rectangle on a screen that is not streaming looks like a
        // failure.
        if (cameraGranted && state.phase == SessionState.Phase.STREAMING) {
            // The viewfinder is a second output of the capture session, not a
            // second camera open. The switch that turns it off is on the Output
            // screen with the other settings, because it is a setting rather than a
            // stream fact; this block only draws the picture once the setting allows
            // it. With the setting off there is no surface at all, so the capture
            // session is built without the preview output.
            if (previewEnabled) {
                // The display correction combines sensor orientation and display rotation.
                val sensorOrientation = state.activeCamera?.orientationDegrees ?: 0
                val displayRotation = getDisplayRotationDegrees(context)
                Box(
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    CameraPreview(
                        onSurfaceAvailable = onSurfaceAvailable,
                        aspectRatio = aspectRatioOf(state.actualResolution, sensorOrientation, displayRotation),
                        mirror = state.mirror,
                        orientationDegrees = sensorOrientation,
                        displayRotationDegrees = displayRotation,
                    )
                    Row(
                        modifier = Modifier
                            .align(Alignment.TopEnd)
                            .padding(12.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Surface(
                            onClick = onEnableBlackout,
                            shape = CircleShape,
                            color = Color(0xBB000000),
                            border = BorderStroke(1.dp, Color(0x44FFFFFF)),
                        ) {
                            Row(
                                modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                Text(text = "🌙", fontSize = 12.sp)
                                Text(
                                    text = "Layar Hitam",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = Color.White,
                                    fontWeight = FontWeight.Medium,
                                )
                            }
                        }

                        Surface(
                            onClick = { onPreviewEnabled(false) },
                            shape = CircleShape,
                            color = Color(0xBB000000),
                            border = BorderStroke(1.dp, Color(0x44FFFFFF)),
                        ) {
                            Row(
                                modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(6.dp),
                            ) {
                                Text(text = "👁️", fontSize = 12.sp)
                                Text(
                                    text = "Matikan Preview",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = Color.White,
                                    fontWeight = FontWeight.Medium,
                                )
                            }
                        }
                    }
                }
            } else {
                PowerSavingCard(
                    onEnablePreview = { onPreviewEnabled(true) },
                    onEnableBlackout = onEnableBlackout,
                    modifier = if (isLandscape) Modifier.padding(horizontal = 16.dp) else Modifier,
                )
            }
        }

        Box(modifier = if (isLandscape) Modifier.padding(horizontal = 16.dp) else Modifier) {
            ConnectionCard(
                state = state,
                isRunning = isRunning,
                pairedServerName = pairedServerName,
                pairedAddress = pairedAddress,
                onScanQr = onScanQr,
                onSearch = onSearch,
                onConnectDiscovered = onConnectDiscovered,
                onConnectManual = onConnectManual,
                onForget = onForget,
            )
        }
    }
}

/**
 * Elegant card displayed when preview is disabled during streaming to save battery and reduce heat.
 */
@Composable
private fun PowerSavingCard(
    onEnablePreview: () -> Unit,
    onEnableBlackout: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(PhoneCamShapes.Panel)
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, PhoneCamShapes.Panel)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(36.dp)
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = "⚡",
                    fontSize = 18.sp,
                )
            }
            Text(
                text = "Preview Dimatikan (Hemat Baterai & Suhu)",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
            )
        }

        Text(
            text = "Kamera tetap streaming ke PC dengan performa penuh. Rendering layar dimatikan untuk menghemat baterai & menjaga suhu HP tetap dingin.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Button(
                onClick = onEnablePreview,
                modifier = Modifier.weight(1f),
            ) {
                Text("Nyalakan Preview")
            }
            OutlinedButton(
                onClick = onEnableBlackout,
                modifier = Modifier.weight(1f),
            ) {
                Text("Layar Hitam (OLED 0-nit)")
            }
        }
    }
}

/** What the phone captures: the camera, and what that camera can do. */
@Composable
private fun CameraScreen(
    state: SessionState,
    onSelectCamera: (String) -> Unit,
) {
    ScreenScroll {
        CameraControlCard(state = state, onSelectCamera = onSelectCamera)
        // Reference material rather than a task, so it sits below the card with
        // no panel of its own.
        CameraCapabilitySection(camera = state.activeCamera)
    }
}

/**
 * What the desktop receives: the stream's shape and the settings it runs with.
 *
 * The phone-preview switch lives here, not on the Status screen, because it is the
 * one control on this screen that the phone owns. Everything else is read-only by
 * protocol (the desktop drives `set_video`, `set_image`, `set_screen`), but whether
 * *this* phone spends a surface, power and heat on a viewfinder is the phone's
 * decision and nobody else's — and it is a decision about cost, which is a fact
 * about the output rather than about whether the stream is working.
 */
@Composable
private fun OutputScreen(
    state: SessionState,
    previewEnabled: Boolean,
    onPreviewEnabled: (Boolean) -> Unit,
    onEnableBlackout: () -> Unit = {},
) {
    ScreenScroll {
        StatusCard(state = state)
        OutputCard(state = state)
        PreviewSettingsCard(
            previewEnabled = previewEnabled,
            streaming = state.isStreaming,
            onPreviewEnabled = onPreviewEnabled,
            onEnableBlackout = onEnableBlackout,
        )
    }
}

/**
 * The phone's own viewfinder, as a setting that can be reached while streaming.
 *
 * It used to be a switch inside the Status screen's preview block, which meant it
 * was only reachable while a stream was running *and* the user happened to be on
 * that screen — the one moment the screen is most likely to be scrolled past the
 * picture. As a setting it is visible and changeable whenever the app is open, and
 * it states what it costs rather than only what it does: the viewfinder is a
 * second camera output, and turning it off is the difference between a warm phone
 * and a cool one over a long stream.
 *
 * It is honest about what it cannot do while a stream is running only in one
 * direction: turning the preview back on mid-stream attaches the surface and
 * rebuilds the capture session, which the camera does without closing the device.
 */
@Composable
private fun PreviewSettingsCard(
    previewEnabled: Boolean,
    streaming: Boolean,
    onPreviewEnabled: (Boolean) -> Unit,
    onEnableBlackout: () -> Unit = {},
) {
    SectionCard(title = stringResource(R.string.section_phone_preview)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = stringResource(R.string.preview_toggle_title),
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.onSurface,
                )
                Text(
                    text = stringResource(R.string.preview_toggle_body),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(checked = previewEnabled, onCheckedChange = onPreviewEnabled)
        }
        // Only while it matters: during a stream, the reassurance that the PC keeps
        // receiving a picture is the question the user is actually asking.
        if (streaming) {
            Hint(stringResource(R.string.preview_toggle_streaming_note))
            OutlinedButton(
                onClick = onEnableBlackout,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 6.dp),
            ) {
                Text("🌙 Layar Hitam (OLED 0-nit)")
            }
        }
    }
}

/** Numbers and the log. */
@Composable
private fun DiagnosticsScreen(
    state: SessionState,
    identity: IdentitySnapshot,
    encoderLabel: String,
    camerasProbed: Boolean,
    appVersion: String,
    logLines: List<String>,
    buildReport: () -> String,
    onRefreshCapabilities: () -> Unit,
    onClearLog: () -> Unit,
) {
    ScreenScroll {
        DiagnosticsCard(
            deviceId = identity.deviceId,
            deviceName = identity.deviceName,
            secretFingerprint = identity.secretFingerprint,
            encoderLabel = encoderLabel,
            cameraCount = state.capabilities.size,
            camerasProbed = camerasProbed,
            appVersion = appVersion,
            logLines = logLines,
            onRefreshCapabilities = onRefreshCapabilities,
            onClearLog = onClearLog,
            buildReport = buildReport,
        )
    }
}

/**
 * OLED Pure-Black Ambient Mode.
 *
 * Draws true-black #000000 (100% OLED off) to completely eliminate screen glow
 * and color casting in dark environments. Displays a dim, minimal digital clock
 * (20% opacity) with subtle minute-by-minute offset for OLED anti-burn-in
 * protection, current battery percentage with icon, and a discrete live status badge.
 * Tapping uncovers controls; tapping again returns to black ambient.
 */
@Composable
private fun BlackoutSurface(state: SessionState, onTap: () -> Unit) {
    var currentTime by remember { mutableStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1000L)
            currentTime = System.currentTimeMillis()
        }
    }

    val calendar = remember(currentTime) {
        Calendar.getInstance().apply { timeInMillis = currentTime }
    }
    val timeFormat = remember { SimpleDateFormat("HH:mm", Locale.getDefault()) }
    val formattedTime = remember(currentTime) { timeFormat.format(Date(currentTime)) }

    // Subtle minute-based offset shifting coordinates slightly every minute to prevent OLED burn-in
    val minute = calendar.get(Calendar.MINUTE)
    val burnInOffsetX = (((minute % 10) - 5) * 3).dp
    val burnInOffsetY = ((((minute / 10) % 6) - 3) * 3).dp

    val ambientColor = Color(0x33FFFFFF) // 20% white opacity
    val liveDotColor = Color(0x66FF3B30) // subtle dim live red

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color(0xFF000000))
            .clickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
                onClick = onTap,
            ),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            modifier = Modifier.offset(x = burnInOffsetX, y = burnInOffsetY),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = formattedTime,
                style = MaterialTheme.typography.displayMedium,
                color = ambientColor,
                fontWeight = FontWeight.Light,
                letterSpacing = 2.sp,
            )

            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                // ● Live status badge
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(5.dp),
                ) {
                    Box(
                        modifier = Modifier
                            .size(6.dp)
                            .clip(CircleShape)
                            .background(liveDotColor),
                    )
                    Text(
                        text = "LIVE",
                        style = MaterialTheme.typography.labelSmall,
                        color = ambientColor,
                        letterSpacing = 1.2.sp,
                        fontWeight = FontWeight.Medium,
                    )
                }

                // Battery percentage with icon
                if (state.batteryPercent >= 0) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Icon(
                            imageVector = if (state.charging) Icons.Default.BatteryChargingFull else Icons.Default.BatteryFull,
                            contentDescription = null,
                            modifier = Modifier.size(13.dp),
                            tint = ambientColor,
                        )
                        Text(
                            text = "${state.batteryPercent}%",
                            style = MaterialTheme.typography.labelSmall,
                            color = ambientColor,
                        )
                    }
                }
            }
        }
    }
}

/**
 * The stream's aspect ratio from its reported size and net orientation.
 *
 * When net rotation is 90 or 270 (portrait), aspect ratio is height/width (9:16).
 * When net rotation is 0 or 180 (landscape), aspect ratio is width/height (16:9).
 */
internal fun aspectRatioOf(
    resolution: String?,
    sensorOrientation: Int = 0,
    displayRotation: Int = 0,
): Float {
    val netRotation = (sensorOrientation - displayRotation + 360) % 360
    val isPortrait = netRotation == 90 || netRotation == 270

    val parts = resolution?.split("x")
    val rawWidth = parts?.getOrNull(0)?.toFloatOrNull()
    val rawHeight = parts?.getOrNull(1)?.toFloatOrNull()

    return if (rawWidth != null && rawHeight != null && rawWidth > 0f && rawHeight > 0f) {
        val width = maxOf(rawWidth, rawHeight)
        val height = minOf(rawWidth, rawHeight)
        if (isPortrait) height / width else width / height
    } else {
        if (isPortrait) 9f / 16f else 16f / 9f
    }
}

/**
 * Resolves current display rotation in degrees (0, 90, 180, 270).
 */
internal fun getDisplayRotationDegrees(context: Context): Int {
    val rotation = runCatching {
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R) {
            context.findActivity()?.display?.rotation ?: context.display?.rotation
        } else {
            @Suppress("DEPRECATION")
            (context.findActivity()?.windowManager ?: context.getSystemService(Context.WINDOW_SERVICE) as? android.view.WindowManager)?.defaultDisplay?.rotation
        }
    }.getOrNull()

    return when (rotation) {
        Surface.ROTATION_90 -> 90
        Surface.ROTATION_180 -> 180
        Surface.ROTATION_270 -> 270
        else -> 0
    }
}

private fun Context.findActivity(): android.app.Activity? {
    var current: Context? = this
    while (current is android.content.ContextWrapper) {
        if (current is android.app.Activity) return current
        current = current.baseContext
    }
    return null
}

/**
 * A calm indicator shown while connecting or reconnecting to a PC.
 *
 * Avoids flashing an alarming red error card when the session is simply
 * searching for or negotiating a connection to the desktop.
 */
@Composable
private fun ConnectingCard(modifier: Modifier = Modifier) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(PhoneCamShapes.Panel)
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, PhoneCamShapes.Panel)
            .padding(16.dp)
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        CircularProgressIndicator(
            modifier = Modifier.size(20.dp),
            strokeWidth = 2.dp,
            color = MaterialTheme.colorScheme.primary,
        )
        Text(
            text = "Searching for PC on local network...",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurface,
        )
    }
}
