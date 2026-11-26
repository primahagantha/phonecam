package com.phonecam.camera.ui.components

import android.content.res.Configuration
import android.graphics.Matrix
import android.graphics.SurfaceTexture
import android.view.Surface
import android.view.TextureView
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.phonecam.camera.ui.theme.PhoneCamShapes
import com.phonecam.camera.ui.theme.PhoneCamStudioTheme

internal const val DEFAULT_STREAM_WIDTH = 1280
internal const val DEFAULT_STREAM_HEIGHT = 720

/**
 * How the camera buffer maps into the preview [TextureView] when their aspect
 * ratios differ. Both modes guarantee 1:1 pixel aspect preservation (zero stretch).
 */
enum class PreviewScaleMode {
    /** Fits the entire rotated camera frame inside the viewport on the obsidian stage. */
    FitCenter,

    /** Fills the viewport edge-to-edge while preserving exact 1:1 aspect ratio. */
    CenterCrop,
}

/**
 * Parses a resolution string such as `"1280x720"` or `"1920x1080"` into
 * landscape-normalised sensor buffer dimensions `(width >= height)`.
 * Falls back to `1280x720` when missing or invalid.
 */
internal fun parseStreamDimensions(resolution: String?): Pair<Int, Int> {
    val parts = resolution?.trim()?.lowercase()?.split("x")
    val w = parts?.getOrNull(0)?.trim()?.toIntOrNull()
    val h = parts?.getOrNull(1)?.trim()?.toIntOrNull()
    return if (w != null && h != null && w > 0 && h > 0) {
        maxOf(w, h) to minOf(w, h)
    } else {
        DEFAULT_STREAM_WIDTH to DEFAULT_STREAM_HEIGHT
    }
}

private class PreviewParamsHolder(
    var streamWidth: Int,
    var streamHeight: Int,
    var orientationDegrees: Int,
    var displayRotationDegrees: Int,
    var mirror: Boolean,
    var scaleMode: PreviewScaleMode,
)

/**
 * The on-screen viewfinder monitor.
 *
 * Displays the exact same Camera2 stream sent to the PC inside an aspect-locked
 * `16:9` obsidian stage (`#06080C`). Both the Compose frame and the underlying
 * [TextureView] transform enforce strict 1:1 pixel geometry so subjects never
 * stretch or squish regardless of view dimensions, stream resolution, or phone
 * rotation (`0° / 90° / 180° / 270°`).
 */
@Composable
fun CameraPreview(
    onSurfaceAvailable: (Surface?) -> Unit,
    modifier: Modifier = Modifier,
    /** Active stream buffer width (landscape sensor space, e.g. 1280 or 1920). */
    streamWidth: Int = DEFAULT_STREAM_WIDTH,
    /** Active stream buffer height (landscape sensor space, e.g. 720 or 1080). */
    streamHeight: Int = DEFAULT_STREAM_HEIGHT,
    /** Container aspect ratio; defaults to true 16:9 studio monitor framing. */
    aspectRatio: Float = 16f / 9f,
    /** Whether the picture should be horizontally mirrored. */
    mirror: Boolean = false,
    /** The camera sensor's orientation in degrees (typically 90 rear, 270 front). */
    orientationDegrees: Int = 0,
    /** The current display rotation in degrees (0, 90, 180, 270). */
    displayRotationDegrees: Int = 0,
    /** Scale mode inside the TextureView. Defaults to FitCenter on obsidian stage. */
    scaleMode: PreviewScaleMode = PreviewScaleMode.FitCenter,
    /** Optional compact overlay anchored inside the 16:9 stage bounds. */
    overlay: @Composable BoxScope.() -> Unit = {},
) {
    val isLandscape = LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    val stageColor = PhoneCamStudioTheme.colors.stageBackground
    val safeBufferWidth = if (streamWidth > 0) streamWidth else DEFAULT_STREAM_WIDTH
    val safeBufferHeight = if (streamHeight > 0) streamHeight else DEFAULT_STREAM_HEIGHT
    val safeAspect = if (aspectRatio > 0f && aspectRatio.isFinite()) aspectRatio else 16f / 9f

    val params = remember {
        PreviewParamsHolder(
            streamWidth = safeBufferWidth,
            streamHeight = safeBufferHeight,
            orientationDegrees = orientationDegrees,
            displayRotationDegrees = displayRotationDegrees,
            mirror = mirror,
            scaleMode = scaleMode,
        )
    }
    params.streamWidth = safeBufferWidth
    params.streamHeight = safeBufferHeight
    params.orientationDegrees = orientationDegrees
    params.displayRotationDegrees = displayRotationDegrees
    params.mirror = mirror
    params.scaleMode = scaleMode

    val stageModifier = if (isLandscape) {
        modifier
            .aspectRatio(safeAspect, matchHeightConstraintsFirst = true)
            .clip(PhoneCamShapes.Stage)
            .background(stageColor)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, PhoneCamShapes.Stage)
    } else {
        modifier
            .fillMaxWidth()
            .aspectRatio(safeAspect)
            .clip(PhoneCamShapes.Stage)
            .background(stageColor)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, PhoneCamShapes.Stage)
    }

    Box(modifier = stageModifier) {
        AndroidView(
            modifier = Modifier.matchParentSize(),
            factory = { viewContext ->
                var currentSurface: Surface? = null
                TextureView(viewContext).apply {
                    // Allow obsidian stage background (#06080C) to show cleanly
                    // around FitCenter frames (e.g., portrait sensor or 4:3 stream).
                    isOpaque = false
                    surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                        override fun onSurfaceTextureAvailable(
                            texture: SurfaceTexture,
                            width: Int,
                            height: Int,
                        ) {
                            texture.setDefaultBufferSize(params.streamWidth, params.streamHeight)
                            setTransform(
                                previewTransform(
                                    sensorOrientation = params.orientationDegrees,
                                    displayRotationDegrees = params.displayRotationDegrees,
                                    mirror = params.mirror,
                                    viewWidth = width,
                                    viewHeight = height,
                                    bufferWidth = params.streamWidth,
                                    bufferHeight = params.streamHeight,
                                    scaleMode = params.scaleMode,
                                ),
                            )
                            runCatching { currentSurface?.release() }
                            val surf = Surface(texture)
                            currentSurface = surf
                            onSurfaceAvailable(surf)
                        }

                        override fun onSurfaceTextureSizeChanged(
                            texture: SurfaceTexture,
                            width: Int,
                            height: Int,
                        ) {
                            texture.setDefaultBufferSize(params.streamWidth, params.streamHeight)
                            setTransform(
                                previewTransform(
                                    sensorOrientation = params.orientationDegrees,
                                    displayRotationDegrees = params.displayRotationDegrees,
                                    mirror = params.mirror,
                                    viewWidth = width,
                                    viewHeight = height,
                                    bufferWidth = params.streamWidth,
                                    bufferHeight = params.streamHeight,
                                    scaleMode = params.scaleMode,
                                ),
                            )
                        }

                        override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
                            onSurfaceAvailable(null)
                            runCatching { currentSurface?.release() }
                            currentSurface = null
                            return true
                        }

                        override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit
                    }
                }
            },
            update = { view ->
                if (view.isAvailable) {
                    view.surfaceTexture?.setDefaultBufferSize(params.streamWidth, params.streamHeight)
                }
                if (view.width > 0 && view.height > 0) {
                    view.setTransform(
                        previewTransform(
                            sensorOrientation = params.orientationDegrees,
                            displayRotationDegrees = params.displayRotationDegrees,
                            mirror = params.mirror,
                            viewWidth = view.width,
                            viewHeight = view.height,
                            bufferWidth = params.streamWidth,
                            bufferHeight = params.streamHeight,
                            scaleMode = params.scaleMode,
                        ),
                    )
                }
            },
        )
        overlay()
    }

    DisposableEffect(Unit) {
        onDispose { onSurfaceAvailable(null) }
    }
}

/**
 * Pure calculation results for the preview transformation, separated for deterministic unit testing.
 */
internal data class PreviewTransformCalculation(
    val netRotation: Int,
    val scaleX: Float,
    val scaleY: Float,
    val centerX: Float,
    val centerY: Float,
    val mirror: Boolean,
)

/**
 * Computes rotation, aspect-preserving scale, and pivot coordinates for a [TextureView].
 *
 * By default Android's [TextureView] maps the entire `[bufferWidth x bufferHeight]`
 * sensor buffer onto `[viewWidth x viewHeight]`, so before any transform a 1x1
 * buffer pixel occupies `(viewWidth / bufferWidth)` horizontally and
 * `(viewHeight / bufferHeight)` vertically.
 *
 * To guarantee zero stretching (every circle in buffer space maps to a true
 * circle on screen), we pick a single uniform screen-pixels-per-buffer-pixel
 * scale `s` — `max` for [PreviewScaleMode.CenterCrop] or `min` for
 * [PreviewScaleMode.FitCenter] — accounting for whether `netRotation` swaps
 * width and height (`90°` or `270°`), and compute:
 *
 *   `scaleX = s * bufferWidth / viewWidth`
 *   `scaleY = s * bufferHeight / viewHeight`
 *
 * After `postScale(scaleX, scaleY)` and the isometric `postRotate(netRotation)`,
 * both buffer axes map to length `s` pixels on screen: zero aspect distortion.
 */
internal fun calculatePreviewTransform(
    sensorOrientation: Int,
    displayRotationDegrees: Int = 0,
    mirror: Boolean = false,
    viewWidth: Int,
    viewHeight: Int,
    bufferWidth: Int = 0,
    bufferHeight: Int = 0,
    scaleMode: PreviewScaleMode = PreviewScaleMode.CenterCrop,
): PreviewTransformCalculation {
    val centerX = if (viewWidth > 0) viewWidth / 2f else 0f
    val centerY = if (viewHeight > 0) viewHeight / 2f else 0f

    val netRotation = if (mirror) {
        ((sensorOrientation + displayRotationDegrees) % 360 + 360) % 360
    } else {
        ((sensorOrientation - displayRotationDegrees) % 360 + 360) % 360
    }

    if (viewWidth <= 0 || viewHeight <= 0) {
        return PreviewTransformCalculation(
            netRotation = netRotation,
            scaleX = 1.0f,
            scaleY = 1.0f,
            centerX = centerX,
            centerY = centerY,
            mirror = mirror,
        )
    }

    val vw = viewWidth.toFloat()
    val vh = viewHeight.toFloat()
    val rotated = netRotation == 90 || netRotation == 270

    // When explicit buffer dimensions are provided (all real camera sessions),
    // normalise to landscape sensor dimensions (width >= height) and enforce
    // strict 1:1 pixel geometry across every view ratio and orientation.
    // When omitted (legacy 0x0 callers), assume the buffer already matches the
    // view's post-rotation aspect ratio.
    val (bw, bh) = if (bufferWidth > 0 && bufferHeight > 0) {
        maxOf(bufferWidth, bufferHeight).toFloat() to minOf(bufferWidth, bufferHeight).toFloat()
    } else if (rotated) {
        vh to vw
    } else {
        vw to vh
    }

    val uniformScale = if (rotated) {
        when (scaleMode) {
            PreviewScaleMode.CenterCrop -> maxOf(vw / bh, vh / bw)
            PreviewScaleMode.FitCenter -> minOf(vw / bh, vh / bw)
        }
    } else {
        when (scaleMode) {
            PreviewScaleMode.CenterCrop -> maxOf(vw / bw, vh / bh)
            PreviewScaleMode.FitCenter -> minOf(vw / bw, vh / bh)
        }
    }

    val scaleX = uniformScale * bw / vw
    val scaleY = uniformScale * bh / vh

    return PreviewTransformCalculation(
        netRotation = netRotation,
        scaleX = scaleX,
        scaleY = scaleY,
        centerX = centerX,
        centerY = centerY,
        mirror = mirror,
    )
}

/**
 * Builds the [TextureView] transform [Matrix] that rotates, scales without
 * aspect distortion, and optionally mirrors the camera preview.
 */
internal fun previewTransform(
    sensorOrientation: Int,
    displayRotationDegrees: Int = 0,
    mirror: Boolean = false,
    viewWidth: Int,
    viewHeight: Int,
    bufferWidth: Int = 0,
    bufferHeight: Int = 0,
    scaleMode: PreviewScaleMode = PreviewScaleMode.CenterCrop,
): Matrix {
    val matrix = Matrix()
    if (viewWidth <= 0 || viewHeight <= 0) return matrix

    val calc = calculatePreviewTransform(
        sensorOrientation = sensorOrientation,
        displayRotationDegrees = displayRotationDegrees,
        mirror = mirror,
        viewWidth = viewWidth,
        viewHeight = viewHeight,
        bufferWidth = bufferWidth,
        bufferHeight = bufferHeight,
        scaleMode = scaleMode,
    )

    matrix.postScale(calc.scaleX, calc.scaleY, calc.centerX, calc.centerY)

    if (calc.netRotation != 0) {
        matrix.postRotate(calc.netRotation.toFloat(), calc.centerX, calc.centerY)
    }

    if (calc.mirror) {
        matrix.postScale(-1f, 1f, calc.centerX, calc.centerY)
    }

    return matrix
}

/** Backward compatibility overload for 4-argument call. */
internal fun previewTransform(
    sensorOrientation: Int,
    mirror: Boolean,
    viewWidth: Int,
    viewHeight: Int,
): Matrix = previewTransform(
    sensorOrientation = sensorOrientation,
    displayRotationDegrees = 0,
    mirror = mirror,
    viewWidth = viewWidth,
    viewHeight = viewHeight,
)
