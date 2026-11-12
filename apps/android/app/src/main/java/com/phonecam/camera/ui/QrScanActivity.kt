package com.phonecam.camera.ui

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import com.google.zxing.ResultPoint
import com.journeyapps.barcodescanner.BarcodeCallback
import com.journeyapps.barcodescanner.BarcodeResult
import com.journeyapps.barcodescanner.DecoratedBarcodeView
import com.phonecam.camera.R
import com.phonecam.camera.nativecore.NativeCore
import com.phonecam.camera.protocol.Log
import com.phonecam.camera.protocol.Proto
import com.phonecam.camera.ui.components.CameraPermissionCard
import com.phonecam.camera.ui.components.PermissionDeniedCard
import com.phonecam.camera.ui.theme.PhoneCamTheme
import com.phonecam.camera.ui.theme.PhoneCamShapes
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/**
 * Scans the pairing code the desktop shows (PRD §9 priority 1).
 *
 * The payload is the `QrPayload` object from `docs/protocol.md` §4.1, validated
 * exactly the way `parseQrPayload` in `@phonecam/shared-types` validates it. A
 * malformed code is rejected here rather than being sent to the PC and refused
 * there, because "that code isn't a PhoneCam pairing code" is far more useful to
 * the person holding the phone than "the PC declined the connection".
 *
 * The scanned token is never logged (§63); only the host and port are.
 *
 * The camera permission is requested here, not assumed from the caller. The
 * scanner used to build its viewfinder unconditionally: on a phone that had never
 * granted the permission the camera open failed silently and the screen stayed
 * black with no prompt, which reads as a broken app rather than a missing switch.
 */
class QrScanActivity : ComponentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            PhoneCamTheme {
                QrScannerScreen(
                    onPayload = { payload -> finishWith(payload) },
                    onRejected = { finishWith(null) },
                    onOpenSettings = { openAppSettings() },
                )
            }
        }
    }

    private fun finishWith(payload: QrPayload?) {
        if (payload == null) {
            // A code was scanned but it was not a PhoneCam pairing code. Kept
            // distinct from the user backing out (RESULT_CANCELED) so the caller
            // can say "that isn't a PhoneCam pairing code" for the one case where
            // it is true, and stay quiet when the user simply closed the scanner.
            setResult(RESULT_REJECTED)
            finish()
            return
        }
        Log.i("pairing code scanned for ${payload.host}:${payload.port}")
        setResult(
            Activity.RESULT_OK,
            Intent().apply {
                putExtra(EXTRA_HOST, payload.host)
                putExtra(EXTRA_PORT, payload.port)
                putExtra(EXTRA_TOKEN, payload.token)
                putExtra(EXTRA_SERVER_NAME, payload.name)
            },
        )
        finish()
    }

    companion object {
        /** The scanner saw a code and it was not a PhoneCam pairing code. */
        const val RESULT_REJECTED = Activity.RESULT_FIRST_USER

        const val EXTRA_HOST = "host"
        const val EXTRA_PORT = "port"
        const val EXTRA_TOKEN = "token"
        const val EXTRA_SERVER_NAME = "serverName"
    }
}

@Composable
private fun QrScannerScreen(
    onPayload: (QrPayload) -> Unit,
    onRejected: () -> Unit,
    onOpenSettings: () -> Unit,
) {
    val context = LocalContext.current

    var granted by remember { mutableStateOf(context.hasCameraPermission()) }
    // Distinguishes "not asked yet" from "asked and refused", which is what
    // decides between prompting again and offering Settings.
    var asked by rememberSaveable { mutableStateOf(false) }

    val launcher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) {
        asked = true
        granted = context.hasCameraPermission()
    }

    // A scanner is only useful with a camera, so the prompt is raised on arrival
    // rather than behind a button: the user already expressed the intent by
    // opening this screen.
    LaunchedEffect(Unit) {
        if (!granted) launcher.launch(context.permissionsToRequest())
    }

    // The camera permission can also change while the scanner is in the
    // background: the user leaves to flip the switch in Settings and comes back.
    // Without this the screen kept the answer it captured at launch — a live
    // viewfinder over a camera it no longer had, or "Camera access is off" after
    // the user had just granted it. MainActivity already re-reads on resume for
    // the same reason; this is the same rule on the one screen that owns the
    // camera directly.
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                granted = context.hasCameraPermission()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    Box(modifier = Modifier.fillMaxSize()) {
        when {
            granted -> QrViewfinder(onPayload = onPayload, onRejected = onRejected)

            // Asked and still refused: the platform will not show the dialog
            // again, so the only honest route left is Settings.
            asked && !context.shouldShowCameraRationale() -> Column(
                modifier = Modifier
                    .align(Alignment.Center)
                    .fillMaxWidth()
                    .padding(24.dp),
            ) {
                PermissionDeniedCard(onOpenSettings = onOpenSettings)
            }

            // The dialog is up, or about to be. Explaining the wait beats an
            // unexplained black screen if the user dismisses it.
            else -> Column(
                modifier = Modifier
                    .align(Alignment.Center)
                    .fillMaxWidth()
                    .padding(24.dp),
            ) {
                CameraPermissionCard(onRequest = { launcher.launch(context.permissionsToRequest()) })
            }
        }

        if (granted) {
            // The prompt sits on an opaque surface, not directly on the viewfinder.
            // This is the one screen where the background is a live camera image, so
            // text drawn straight onto it has no guaranteed contrast: a light wall
            // behind the code turns `onBackground` white-on-white. The panel makes the
            // contrast a property of the theme again, whatever the camera is pointed
            // at.
            Column(
                modifier = Modifier
                    .align(Alignment.TopCenter)
                    .fillMaxWidth()
                    .padding(24.dp)
                    .clip(PhoneCamShapes.Panel)
                    .background(MaterialTheme.colorScheme.surface)
                    .border(1.dp, MaterialTheme.colorScheme.outlineVariant, PhoneCamShapes.Panel)
                    .padding(16.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Text(
                    text = stringResource(R.string.qr_scan_prompt),
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    textAlign = TextAlign.Center,
                )
                Text(
                    text = stringResource(R.string.app_tagline),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }
        }
    }
}

/** The live viewfinder, which is only built once the camera is ours to open. */
@Composable
private fun QrViewfinder(
    onPayload: (QrPayload) -> Unit,
    onRejected: () -> Unit,
) {
    // A viewfinder is the one place where a stale frame misleads: the code is
    // scanned once and then the screen stops looking at the camera.
    var handled by remember { mutableStateOf(false) }
    var view by remember { mutableStateOf<DecoratedBarcodeView?>(null) }

    AndroidView(
        modifier = Modifier.fillMaxSize(),
        factory = { viewContext ->
            DecoratedBarcodeView(viewContext).apply {
                decodeContinuous(
                    object : BarcodeCallback {
                        override fun barcodeResult(result: BarcodeResult?) {
                            if (handled || result == null) return
                            handled = true
                            val parsed = parseQrPayload(result.text)
                            if (parsed == null) onRejected() else onPayload(parsed)
                        }

                        override fun possibleResultPoints(resultPoints: MutableList<ResultPoint>?) = Unit
                    },
                )
                view = this
            }
        },
    )

    // The camera behind the viewfinder is a resource the rest of the phone shares.
    // Leaving it open while this screen is not on top would block the streaming
    // session, so it is released on pause and taken back on resume.
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, view) {
        val barcodeView = view ?: return@DisposableEffect onDispose {}
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_RESUME -> runCatching { barcodeView.resume() }
                Lifecycle.Event.ON_PAUSE -> runCatching { barcodeView.pause() }
                else -> Unit
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
            runCatching { barcodeView.pause() }
        }
    }
}

/** The parsed contents of a desktop pairing code (docs/protocol.md §4.1). */
data class QrPayload(
    val host: String,
    val port: Int,
    val token: String,
    val name: String?,
    val serverId: String?,
)

@Serializable
private data class NormalizedQrPayload(
    val protocol: String,
    val version: Int,
    val host: String,
    val port: Int,
    val token: String,
    val name: String? = null,
    val serverId: String? = null,
)

private val qrJson = Json { ignoreUnknownKeys = true }

/**
 * Parses a scanned string via the shared Rust core; returns `null` for anything
 * that is not a PhoneCam pairing code, including JSON that happens to be well formed.
 */
fun parseQrPayload(raw: String): QrPayload? {
    val normalized = NativeCore.validateQrPayload(raw) ?: return null
    val parsed = runCatching {
        qrJson.decodeFromString(NormalizedQrPayload.serializer(), normalized)
    }.getOrNull() ?: return null

    return QrPayload(
        host = parsed.host,
        port = parsed.port,
        token = parsed.token,
        name = parsed.name?.takeIf { it.isNotBlank() },
        serverId = parsed.serverId?.takeIf { it.isNotBlank() },
    )
}
