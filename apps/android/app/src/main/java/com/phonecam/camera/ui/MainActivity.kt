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
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.BatteryChargingFull
import androidx.compose.material.icons.filled.BatteryFull
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.DarkMode
import androidx.compose.material.icons.filled.MonitorHeart
import androidx.compose.material.icons.filled.Output
import androidx.compose.material.icons.filled.PhotoCamera
import androidx.compose.material.icons.filled.Troubleshoot
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarDefaults
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.NavigationRailItemDefaults
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
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
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
import com.phonecam.camera.ui.components.CameraCapabilitySection
import com.phonecam.camera.ui.components.CameraControlCard
import com.phonecam.camera.ui.components.CameraPermissionCard
import com.phonecam.camera.ui.components.CameraPreview
import com.phonecam.camera.ui.components.ConnectionCard
import com.phonecam.camera.ui.components.DiagnosticsCard
import com.phonecam.camera.ui.components.ErrorCard
import com.phonecam.camera.ui.components.Hairline
import com.phonecam.camera.ui.components.HealthCard
import com.phonecam.camera.ui.components.Hint
import com.phonecam.camera.ui.components.InfoBanner
import com.phonecam.camera.ui.components.InsetWell
import com.phonecam.camera.ui.components.NotificationPermissionNote
import com.phonecam.camera.ui.components.OutputCard
import com.phonecam.camera.ui.components.PermissionDeniedCard
import com.phonecam.camera.ui.components.PreviewScaleMode
import com.phonecam.camera.ui.components.SectionCard
import com.phonecam.camera.ui.components.StatusCard
import com.phonecam.camera.ui.components.StudioTopBar
import com.phonecam.camera.ui.components.UpdateCard
import com.phonecam.camera.ui.components.VerticalHairline
import com.phonecam.camera.ui.components.parseStreamDimensions
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.PhoneCamStudioTheme
import com.phonecam.camera.ui.theme.PhoneCamTheme
import com.phonecam.camera.update.UpdateCheckState
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.delay

/**
 * PhoneCam Android Studio Control Panel.
 *
 * Adapts cleanly between Portrait (compact Studio Top Bar + crisp 16:9 preview +
 * bottom NavigationBar) and Landscape (68dp left NavigationRail + 44dp compact
 * Studio Top Bar + full-height two-column live monitor & scrollable controls).
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
    val isLandscape = LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE

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
    val updateState by viewModel.updateState.collectAsState()

    val screenMode = ScreenMode.fromWire(state.screenMode)
    var localBlackoutActive by rememberSaveable { mutableStateOf(false) }
    var blackoutDismissed by rememberSaveable(state.screenMode) { mutableStateOf(false) }

    LaunchedEffect(state.phase) {
        if (state.phase != SessionState.Phase.STREAMING) {
            localBlackoutActive = false
            blackoutDismissed = false
        }
    }

    val isStreaming = state.phase == SessionState.Phase.STREAMING
    val isBlackoutMode = !blackoutDismissed && (screenMode == ScreenMode.BLACK || localBlackoutActive)
    val effectiveScreenMode = when {
        localBlackoutActive -> ScreenMode.BLACK
        blackoutDismissed && screenMode == ScreenMode.BLACK -> ScreenMode.KEEP_ON
        else -> screenMode
    }
    ApplyScreenMode(
        mode = effectiveScreenMode,
        active = isStreaming,
    )

    var blackoutPeeked by remember(screenMode, localBlackoutActive, blackoutDismissed) { mutableStateOf(false) }
    var blackoutInteractionTick by remember { mutableIntStateOf(0) }

    LaunchedEffect(isStreaming, isBlackoutMode, blackoutPeeked, blackoutInteractionTick) {
        if (isStreaming && isBlackoutMode && blackoutPeeked) {
            delay(5_000L)
            blackoutPeeked = false
        }
    }

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
            viewModel.startSession()
        }
    }

    val scanLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.StartActivityForResult(),
    ) { result ->
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
            viewModel.reportQrDispatched()
        }
    }

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
    val enableBlackout: () -> Unit = {
        localBlackoutActive = true
        blackoutDismissed = false
        blackoutPeeked = false
    }

    val encoderLabel = stringResource(
        when (hasHardwareEncoder) {
            true -> R.string.diagnostics_encoder_hardware
            false -> R.string.diagnostics_encoder_software
            null -> R.string.diagnostics_not_detected
        },
    )

    val buildReport: () -> String = {
        BugReport.build(
            appVersion = viewModel.appVersion,
            deviceId = identity.deviceId,
            deviceName = identity.deviceName,
            device = ReportDevice.current(),
            session = ReportSession(
                phase = state.phase.name,
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

    var pendingCrashReport by remember {
        mutableStateOf(com.phonecam.camera.protocol.CrashReporter.readPendingCrashReport(context))
    }
    if (pendingCrashReport != null) {
        val report = pendingCrashReport!!
        val crashClipLabel = stringResource(R.string.clipboard_crash_report)
        AlertDialog(
            onDismissRequest = {
                com.phonecam.camera.protocol.CrashReporter.clearPendingCrashReport(context)
                pendingCrashReport = null
            },
            containerColor = MaterialTheme.colorScheme.surface,
            title = { Text(stringResource(R.string.crash_dialog_title)) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Text(
                        stringResource(R.string.crash_dialog_body),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    InsetWell {
                        Text(
                            report.take(300) + if (report.length > 300) "..." else "",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            },
            confirmButton = {
                Button(
                    onClick = {
                        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        clipboard.setPrimaryClip(ClipData.newPlainText(crashClipLabel, report))
                        val intent = com.phonecam.camera.protocol.CrashReporter.buildGitHubIssueIntent(
                            context = context,
                            title = "[Crash] Android App Crash Report",
                            bodyMarkdown = "### Crash Report\n```\n$report\n```",
                        )
                        runCatching { context.startActivity(intent) }
                        com.phonecam.camera.protocol.CrashReporter.clearPendingCrashReport(context)
                        pendingCrashReport = null
                    },
                    shape = PhoneCamShapes.Chip,
                ) {
                    Text(stringResource(R.string.action_report_github))
                }
            },
            dismissButton = {
                TextButton(
                    onClick = {
                        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        clipboard.setPrimaryClip(ClipData.newPlainText(crashClipLabel, report))
                        com.phonecam.camera.protocol.CrashReporter.clearPendingCrashReport(context)
                        pendingCrashReport = null
                    },
                    shape = PhoneCamShapes.Chip,
                ) {
                    Text(stringResource(R.string.action_copy_dismiss))
                }
            },
        )
    }

    val renderDestination: @Composable () -> Unit = {
        when (destination) {
            ScreenDestination.Status -> StatusScreen(
                state = state,
                isRunning = isRunning,
                isLandscape = isLandscape,
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
                onEnableBlackout = enableBlackout,
            )

            ScreenDestination.Camera -> CameraScreen(
                state = state,
                isLandscape = isLandscape,
                onSelectCamera = { cameraId -> viewModel.selectCamera(cameraId) },
            )

            ScreenDestination.Output -> OutputScreen(
                state = state,
                isLandscape = isLandscape,
                previewEnabled = previewEnabled,
                onPreviewEnabled = { enabled -> viewModel.setPreviewEnabled(enabled) },
                onEnableBlackout = enableBlackout,
            )

            ScreenDestination.Diagnostics -> DiagnosticsScreen(
                state = state,
                isLandscape = isLandscape,
                identity = identity,
                encoderLabel = encoderLabel,
                camerasProbed = camerasProbed,
                appVersion = viewModel.appVersion,
                updateState = updateState,
                onCheckForUpdates = { viewModel.checkForUpdates() },
                logLines = logLines,
                buildReport = buildReport,
                onRefreshCapabilities = { viewModel.refreshCapabilities() },
                onClearLog = { viewModel.clearLog() },
            )
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .pointerInput(isStreaming, isBlackoutMode, blackoutPeeked) {
                if (isStreaming && isBlackoutMode && blackoutPeeked) {
                    awaitPointerEventScope {
                        while (true) {
                            awaitPointerEvent(PointerEventPass.Initial)
                            blackoutInteractionTick++
                        }
                    }
                }
            },
    ) {
        if (isLandscape) {
            // Adaptive Landscape Studio Shell:
            // Sleek 68dp left NavigationRail + 44dp top StudioTopBar = +100dp more
            // vertical room than stacked bottom chrome, so nothing is ever clipped.
            Scaffold(
                snackbarHost = { SnackbarHost(snackbarHost) },
                containerColor = MaterialTheme.colorScheme.background,
                contentWindowInsets = WindowInsets(0, 0, 0, 0),
            ) { insets ->
                Row(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(insets),
                ) {
                    StudioNavigationRail(
                        selected = destination,
                        onSelect = { destinationOrdinal = it.ordinal },
                    )
                    VerticalHairline()
                    Column(
                        modifier = Modifier
                            .weight(1f)
                            .fillMaxHeight(),
                    ) {
                        StudioTopBar(
                            state = state,
                            isRunning = isRunning,
                            isLandscape = true,
                            onStart = startOrRequestCamera,
                            onStop = { viewModel.stopSession() },
                            modifier = Modifier.windowInsetsPadding(
                                WindowInsets.safeDrawing.only(WindowInsetsSides.End),
                            ),
                        )
                        Box(
                            modifier = Modifier
                                .weight(1f)
                                .fillMaxWidth()
                                .windowInsetsPadding(
                                    WindowInsets.safeDrawing.only(
                                        WindowInsetsSides.End + WindowInsetsSides.Bottom,
                                    ),
                                ),
                        ) {
                            renderDestination()
                        }
                    }
                }
            }
        } else {
            // Clean Portrait Studio Shell:
            // Compact StudioTopBar at top, generous scrolling body, clean NavigationBar at bottom.
            Scaffold(
                snackbarHost = { SnackbarHost(snackbarHost) },
                containerColor = MaterialTheme.colorScheme.background,
                contentWindowInsets = WindowInsets(0, 0, 0, 0),
                topBar = {
                    StudioTopBar(
                        state = state,
                        isRunning = isRunning,
                        isLandscape = false,
                        onStart = startOrRequestCamera,
                        onStop = { viewModel.stopSession() },
                    )
                },
                bottomBar = {
                    StudioBottomBar(
                        selected = destination,
                        onSelect = { destinationOrdinal = it.ordinal },
                    )
                },
            ) { insets ->
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(insets),
                ) {
                    renderDestination()
                }
            }
        }

        if (isStreaming && isBlackoutMode && blackoutPeeked) {
            BlackoutPeekBar(
                onRedim = { blackoutPeeked = false },
                onExitBlackout = {
                    localBlackoutActive = false
                    blackoutDismissed = true
                    blackoutPeeked = false
                },
                modifier = Modifier
                    .align(Alignment.TopCenter)
                    .statusBarsPadding()
                    .padding(top = 12.dp, start = 16.dp, end = 16.dp),
            )
        }

        if (isStreaming && isBlackoutMode && !blackoutPeeked) {
            BlackoutSurface(state = state, onTap = { blackoutPeeked = true })
        }
    }
}

@Composable
private fun StudioNavigationRail(
    selected: ScreenDestination,
    onSelect: (ScreenDestination) -> Unit,
) {
    NavigationRail(
        modifier = Modifier
            .fillMaxHeight()
            .width(68.dp)
            .background(MaterialTheme.colorScheme.surface)
            .windowInsetsPadding(
                WindowInsets.safeDrawing.only(WindowInsetsSides.Start + WindowInsetsSides.Vertical),
            ),
        containerColor = MaterialTheme.colorScheme.surface,
        contentColor = MaterialTheme.colorScheme.onSurface,
        windowInsets = WindowInsets(0, 0, 0, 0),
    ) {
        Spacer(Modifier.weight(1f))
        ScreenDestination.entries.forEach { item ->
            NavigationRailItem(
                selected = item == selected,
                onClick = { onSelect(item) },
                colors = NavigationRailItemDefaults.colors(
                    selectedIconColor = MaterialTheme.colorScheme.onPrimaryContainer,
                    selectedTextColor = MaterialTheme.colorScheme.onSurface,
                    indicatorColor = MaterialTheme.colorScheme.primaryContainer,
                    unselectedIconColor = MaterialTheme.colorScheme.onSurfaceVariant,
                    unselectedTextColor = MaterialTheme.colorScheme.onSurfaceVariant,
                ),
                icon = {
                    Icon(
                        imageVector = item.icon,
                        contentDescription = stringResource(item.labelRes),
                        modifier = Modifier.size(20.dp),
                    )
                },
                label = {
                    Text(
                        text = stringResource(item.labelRes),
                        style = MaterialTheme.typography.labelSmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
            )
        }
        Spacer(Modifier.weight(1f))
    }
}

@Composable
private fun StudioBottomBar(
    selected: ScreenDestination,
    onSelect: (ScreenDestination) -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Hairline()
        NavigationBar(
            containerColor = MaterialTheme.colorScheme.surface,
            tonalElevation = 0.dp,
            windowInsets = NavigationBarDefaults.windowInsets,
        ) {
            ScreenDestination.entries.forEach { item ->
                NavigationBarItem(
                    selected = item == selected,
                    onClick = { onSelect(item) },
                    colors = NavigationBarItemDefaults.colors(
                        selectedIconColor = MaterialTheme.colorScheme.onPrimaryContainer,
                        selectedTextColor = MaterialTheme.colorScheme.onSurface,
                        indicatorColor = MaterialTheme.colorScheme.primaryContainer,
                        unselectedIconColor = MaterialTheme.colorScheme.onSurfaceVariant,
                        unselectedTextColor = MaterialTheme.colorScheme.onSurfaceVariant,
                    ),
                    icon = {
                        Icon(
                            imageVector = item.icon,
                            contentDescription = null,
                        )
                    },
                    label = {
                        Text(
                            text = stringResource(item.labelRes),
                            style = MaterialTheme.typography.labelSmall,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    },
                )
            }
        }
    }
}

@Composable
private fun BlackoutPeekBar(
    onRedim: () -> Unit,
    onExitBlackout: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier,
        shape = PhoneCamShapes.Control,
        color = MaterialTheme.colorScheme.inverseSurface,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
        shadowElevation = 4.dp,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            TextButton(onClick = onRedim) {
                Text(
                    text = stringResource(R.string.blackout_redim),
                    color = MaterialTheme.colorScheme.inverseOnSurface,
                )
            }
            TextButton(onClick = onExitBlackout) {
                Text(
                    text = stringResource(R.string.blackout_exit),
                    color = MaterialTheme.colorScheme.inversePrimary,
                )
            }
        }
    }
}

@Composable
private fun ScreenScroll(
    modifier: Modifier = Modifier,
    horizontalPadding: androidx.compose.ui.unit.Dp = 16.dp,
    maxWidth: androidx.compose.ui.unit.Dp = 720.dp,
    content: @Composable ColumnScope.() -> Unit,
) {
    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.TopCenter,
    ) {
        Column(
            modifier = modifier
                .fillMaxSize()
                .widthIn(max = maxWidth)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = horizontalPadding, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            content = content,
        )
    }
}

@Composable
private fun StatusBanners(
    state: SessionState,
    cameraGranted: Boolean,
    notificationGranted: Boolean,
    notificationNoteDismissed: Boolean,
    permissionRequested: Boolean,
    notice: String?,
    noticeKind: NoticeKind,
    buildReport: () -> String,
    onRequestCamera: () -> Unit,
    onDismissNotificationNote: () -> Unit,
    onRetry: () -> Unit,
    onScanQr: () -> Unit,
    onAcknowledgeNotice: () -> Unit,
) {
    val context = LocalContext.current

    if (!cameraGranted) {
        if (permissionRequested && !context.shouldShowCameraRationale()) {
            PermissionDeniedCard(onOpenSettings = { context.openAppSettings() })
        } else {
            CameraPermissionCard(onRequest = onRequestCamera)
        }
    } else if (!notificationGranted && !notificationNoteDismissed) {
        NotificationPermissionNote(onDismiss = onDismissNotificationNote)
    }

    if (state.phase == SessionState.Phase.ERROR && state.error != null) {
        ErrorCard(
            message = state.error,
            onRetry = onRetry,
            secondaryLabel = stringResource(R.string.action_scan_qr),
            onSecondary = onScanQr,
            onReportIssue = {
                val reportText = buildReport()
                val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                clipboard.setPrimaryClip(
                    ClipData.newPlainText(
                        context.getString(R.string.clipboard_diagnostics_report),
                        reportText,
                    ),
                )
                val intent = com.phonecam.camera.protocol.CrashReporter.buildGitHubIssueIntent(
                    context = context,
                    title = "[Error] Stream error: ${state.error}",
                    bodyMarkdown = "### Error Message\n${state.error}\n\n### System Diagnostics\n```\n$reportText\n```",
                )
                runCatching { context.startActivity(intent) }
            },
        )
    } else if (state.phase == SessionState.Phase.CONNECTING || state.phase == SessionState.Phase.RECONNECTING) {
        ConnectingCard()
    }

    notice?.let { text ->
        val title = when (noticeKind) {
            NoticeKind.Approval -> stringResource(R.string.banner_approval_title)
            NoticeKind.Substitution -> stringResource(R.string.banner_notice_title)
        }
        InfoBanner(text = text, title = title, onDismiss = onAcknowledgeNotice)
    }

    HealthCard(state = state)
}

@Composable
private fun StatusScreen(
    state: SessionState,
    isRunning: Boolean,
    isLandscape: Boolean,
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
    val showStreamPanel = cameraGranted && state.phase == SessionState.Phase.STREAMING

    if (isLandscape && showStreamPanel) {
        // Two-column Landscape Studio layout:
        // Left (weight 1.25f): full-height undistorted 16:9 camera stage.
        // Right (weight 1f): independently scrollable banners + ConnectionCard.
        Row(
            modifier = Modifier
                .fillMaxSize()
                .padding(horizontal = 14.dp, vertical = 10.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                modifier = Modifier
                    .weight(1.25f)
                    .fillMaxHeight(),
                contentAlignment = Alignment.Center,
            ) {
                StreamPreviewOrSaving(
                    state = state,
                    previewEnabled = previewEnabled,
                    onSurfaceAvailable = onSurfaceAvailable,
                    onPreviewEnabled = onPreviewEnabled,
                    onEnableBlackout = onEnableBlackout,
                )
            }
            Column(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxHeight()
                    .verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                StatusBanners(
                    state = state,
                    cameraGranted = cameraGranted,
                    notificationGranted = notificationGranted,
                    notificationNoteDismissed = notificationNoteDismissed,
                    permissionRequested = permissionRequested,
                    notice = notice,
                    noticeKind = noticeKind,
                    buildReport = buildReport,
                    onRequestCamera = onRequestCamera,
                    onDismissNotificationNote = onDismissNotificationNote,
                    onRetry = onRetry,
                    onScanQr = onScanQr,
                    onAcknowledgeNotice = onAcknowledgeNotice,
                )
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
    } else {
        ScreenScroll {
            StatusBanners(
                state = state,
                cameraGranted = cameraGranted,
                notificationGranted = notificationGranted,
                notificationNoteDismissed = notificationNoteDismissed,
                permissionRequested = permissionRequested,
                notice = notice,
                noticeKind = noticeKind,
                buildReport = buildReport,
                onRequestCamera = onRequestCamera,
                onDismissNotificationNote = onDismissNotificationNote,
                onRetry = onRetry,
                onScanQr = onScanQr,
                onAcknowledgeNotice = onAcknowledgeNotice,
            )

            if (showStreamPanel) {
                StreamPreviewOrSaving(
                    state = state,
                    previewEnabled = previewEnabled,
                    onSurfaceAvailable = onSurfaceAvailable,
                    onPreviewEnabled = onPreviewEnabled,
                    onEnableBlackout = onEnableBlackout,
                )
            }

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

@Composable
private fun StreamPreviewOrSaving(
    state: SessionState,
    previewEnabled: Boolean,
    onSurfaceAvailable: (Surface?) -> Unit,
    onPreviewEnabled: (Boolean) -> Unit,
    onEnableBlackout: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    if (previewEnabled) {
        val sensorOrientation = state.activeCamera?.orientationDegrees ?: 0
        val displayRotation = getDisplayRotationDegrees(context)
        val (streamWidth, streamHeight) = parseStreamDimensions(
            state.actualResolution ?: state.requestedResolution,
        )

        CameraPreview(
            onSurfaceAvailable = onSurfaceAvailable,
            modifier = modifier,
            streamWidth = streamWidth,
            streamHeight = streamHeight,
            aspectRatio = 16f / 9f,
            mirror = state.mirror,
            orientationDegrees = sensorOrientation,
            displayRotationDegrees = displayRotation,
            scaleMode = PreviewScaleMode.FitCenter,
            overlay = {
                // Minimal semi-transparent studio controls anchored cleanly at bottom-right
                // so the top center of the frame stays completely unobstructed.
                Row(
                    modifier = Modifier
                        .align(Alignment.BottomEnd)
                        .padding(8.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    PreviewQuickChip(
                        icon = Icons.Filled.DarkMode,
                        label = stringResource(R.string.action_oled_short),
                        contentDescription = stringResource(R.string.action_blackout_oled),
                        onClick = onEnableBlackout,
                    )
                    PreviewQuickChip(
                        icon = Icons.Filled.VisibilityOff,
                        label = stringResource(R.string.action_hide_short),
                        contentDescription = stringResource(R.string.action_hide_preview),
                        onClick = { onPreviewEnabled(false) },
                    )
                }
            },
        )
    } else {
        PowerSavingCard(
            onEnablePreview = { onPreviewEnabled(true) },
            onEnableBlackout = onEnableBlackout,
            modifier = modifier,
        )
    }
}

@Composable
private fun PreviewQuickChip(
    icon: ImageVector,
    label: String,
    contentDescription: String,
    onClick: () -> Unit,
) {
    Surface(
        onClick = onClick,
        shape = PhoneCamShapes.Chip,
        color = Color(0xCC090C10),
        border = BorderStroke(1.dp, Color(0x38FFFFFF)),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 9.dp, vertical = 5.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            Icon(
                imageVector = icon,
                contentDescription = contentDescription,
                tint = Color(0xFFF1F5F9),
                modifier = Modifier.size(14.dp),
            )
            Text(
                text = label,
                style = MaterialTheme.typography.labelSmall,
                color = Color(0xFFF1F5F9),
                fontWeight = FontWeight.Medium,
                maxLines = 1,
            )
        }
    }
}

@Composable
private fun PowerSavingCard(
    onEnablePreview: () -> Unit,
    onEnableBlackout: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(PhoneCamShapes.Stage)
            .background(MaterialTheme.colorScheme.surface)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, PhoneCamShapes.Stage)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(34.dp)
                    .clip(PhoneCamShapes.Chip)
                    .background(MaterialTheme.colorScheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Filled.Bolt,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onPrimaryContainer,
                    modifier = Modifier.size(18.dp),
                )
            }
            Text(
                text = stringResource(R.string.power_saving_title),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.weight(1f),
            )
        }

        Text(
            text = stringResource(R.string.power_saving_body),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Button(
                onClick = onEnablePreview,
                shape = PhoneCamShapes.Chip,
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 9.dp),
                modifier = Modifier.weight(1f),
            ) {
                Text(
                    text = stringResource(R.string.action_show_preview),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            OutlinedButton(
                onClick = onEnableBlackout,
                shape = PhoneCamShapes.Chip,
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 9.dp),
                modifier = Modifier.weight(1f),
            ) {
                Text(
                    text = stringResource(R.string.action_blackout),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun CameraScreen(
    state: SessionState,
    isLandscape: Boolean,
    onSelectCamera: (String) -> Unit,
) {
    if (isLandscape) {
        Row(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 14.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.Top,
        ) {
            CameraControlCard(
                state = state,
                onSelectCamera = onSelectCamera,
                modifier = Modifier.weight(1f),
            )
            CameraCapabilitySection(
                camera = state.activeCamera,
                modifier = Modifier.weight(1.1f),
            )
        }
    } else {
        ScreenScroll {
            CameraControlCard(state = state, onSelectCamera = onSelectCamera)
            CameraCapabilitySection(camera = state.activeCamera)
        }
    }
}

@Composable
private fun OutputScreen(
    state: SessionState,
    isLandscape: Boolean,
    previewEnabled: Boolean,
    onPreviewEnabled: (Boolean) -> Unit,
    onEnableBlackout: () -> Unit = {},
) {
    if (isLandscape) {
        Row(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 14.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.Top,
        ) {
            StatusCard(state = state, modifier = Modifier.weight(1f))
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OutputCard(state = state)
                PreviewSettingsCard(
                    previewEnabled = previewEnabled,
                    streaming = state.isStreaming,
                    onPreviewEnabled = onPreviewEnabled,
                    onEnableBlackout = onEnableBlackout,
                )
            }
        }
    } else {
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
}

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
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
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
        if (streaming) {
            Hint(stringResource(R.string.preview_toggle_streaming_note))
            OutlinedButton(
                onClick = onEnableBlackout,
                shape = PhoneCamShapes.Field,
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 4.dp),
            ) {
                Text(stringResource(R.string.action_blackout_oled))
            }
        }
    }
}

@Composable
private fun DiagnosticsScreen(
    state: SessionState,
    isLandscape: Boolean,
    identity: IdentitySnapshot,
    encoderLabel: String,
    camerasProbed: Boolean,
    appVersion: String,
    updateState: UpdateCheckState,
    onCheckForUpdates: () -> Unit,
    logLines: List<String>,
    buildReport: () -> String,
    onRefreshCapabilities: () -> Unit,
    onClearLog: () -> Unit,
) {
    if (isLandscape) {
        Row(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 14.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.Top,
        ) {
            UpdateCard(
                appVersion = appVersion,
                updateState = updateState,
                onCheckForUpdates = onCheckForUpdates,
                modifier = Modifier.weight(1f),
            )
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
                modifier = Modifier.weight(1.15f),
            )
        }
    } else {
        ScreenScroll {
            UpdateCard(
                appVersion = appVersion,
                updateState = updateState,
                onCheckForUpdates = onCheckForUpdates,
            )
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
}

/**
 * OLED Pure-Black Ambient Mode (`#000000`, 0-nit) with minute-by-minute
 * anti-burn-in coordinate drift and tap-to-peek controls.
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

    val minute = calendar.get(Calendar.MINUTE)
    val burnInOffsetX = (((minute % 10) - 5) * 3).dp
    val burnInOffsetY = ((((minute / 10) % 6) - 3) * 3).dp

    val ambientColor = Color(0x33FFFFFF)
    val liveDotColor = PhoneCamStudioTheme.colors.live.copy(alpha = 0.55f)

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
                style = MaterialTheme.typography.displayMedium.copy(fontFeatureSettings = "tnum"),
                color = ambientColor,
                fontWeight = FontWeight.Light,
                letterSpacing = 2.sp,
            )

            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(14.dp),
            ) {
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
                        text = stringResource(R.string.blackout_live_badge),
                        style = MaterialTheme.typography.labelSmall,
                        color = ambientColor,
                        letterSpacing = 1.2.sp,
                        fontWeight = FontWeight.Medium,
                    )
                }

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
                            style = MaterialTheme.typography.labelSmall.copy(fontFeatureSettings = "tnum"),
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

/** Resolves current display rotation in degrees (0, 90, 180, 270). */
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

@Composable
private fun ConnectingCard(modifier: Modifier = Modifier) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(PhoneCamShapes.Panel)
            .background(MaterialTheme.colorScheme.surface)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, PhoneCamShapes.Panel)
            .padding(16.dp)
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        CircularProgressIndicator(
            modifier = Modifier.size(18.dp),
            strokeWidth = 2.dp,
            color = MaterialTheme.colorScheme.primary,
        )
        Text(
            text = stringResource(R.string.connecting_searching_lan),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurface,
        )
    }
}
