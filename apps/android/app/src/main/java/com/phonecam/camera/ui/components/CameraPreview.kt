package com.phonecam.camera.ui.components

import android.content.res.Configuration
import android.graphics.Matrix
import android.graphics.SurfaceTexture
import android.view.Surface
import android.view.TextureView
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.phonecam.camera.ui.theme.PhoneCamShapes

/**
 * The on-screen viewfinder.
 *
 * This exists because a webcam with no picture is indistinguishable from a broken
 * one: before it, the only evidence the camera was working was a resolution
 * string on another device's screen. It shows the same frames the PC receives,
 * because it is a second output of the same capture session, not a second camera
 * open.
 *
 * The surface is handed out through [onSurfaceAvailable] and withdrawn with
 * `null` when the view goes away, so the camera session can drop the extra output
 * rather than holding a dead buffer.
 *
 * The display correction combines the camera's sensor orientation and current
 * display rotation to produce a clean, letterbox-free and stretch-free viewfinder.
 */
@Composable
fun CameraPreview(
    onSurfaceAvailable: (Surface?) -> Unit,
    modifier: Modifier = Modifier,
    /** The stream's aspect ratio, so the viewfinder is not letterboxed. */
    aspectRatio: Float = 16f / 9f,
    /** Whether the picture should be mirrored. */
    mirror: Boolean = false,
    /** The camera sensor's orientation, for the display correction. */
    orientationDegrees: Int = 0,
    /** The current display rotation in degrees (0, 90, 180, 270). */
    displayRotationDegrees: Int = 0,
) {
    val isLandscape = LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    val sizeModifier = if (isLandscape) {
        modifier
            .heightIn(max = 220.dp)
            .aspectRatio(aspectRatio)
            .clip(PhoneCamShapes.Panel)
    } else {
        modifier
            .fillMaxWidth()
            .aspectRatio(aspectRatio)
            .clip(PhoneCamShapes.Panel)
    }

    Box(modifier = sizeModifier) {
        AndroidView(
            modifier = Modifier.matchParentSize(),
            factory = { viewContext ->
                var currentSurface: Surface? = null
                TextureView(viewContext).apply {
                    surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                        override fun onSurfaceTextureAvailable(
                            texture: SurfaceTexture,
                            width: Int,
                            height: Int,
                        ) {
                            setTransform(
                                previewTransform(
                                    sensorOrientation = orientationDegrees,
                                    displayRotationDegrees = displayRotationDegrees,
                                    mirror = mirror,
                                    viewWidth = width,
                                    viewHeight = height,
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
                            setTransform(
                                previewTransform(
                                    sensorOrientation = orientationDegrees,
                                    displayRotationDegrees = displayRotationDegrees,
                                    mirror = mirror,
                                    viewWidth = width,
                                    viewHeight = height,
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
                view.setTransform(
                    previewTransform(
                        sensorOrientation = orientationDegrees,
                        displayRotationDegrees = displayRotationDegrees,
                        mirror = mirror,
                        viewWidth = view.width,
                        viewHeight = view.height,
                    ),
                )
            },
        )
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
 * Computes rotation, scaling, and center coordinates for the preview transform.
 */
internal fun calculatePreviewTransform(
    sensorOrientation: Int,
    displayRotationDegrees: Int = 0,
    mirror: Boolean = false,
    viewWidth: Int,
    viewHeight: Int,
): PreviewTransformCalculation {
    val centerX = if (viewWidth > 0) viewWidth / 2f else 0f
    val centerY = if (viewHeight > 0) viewHeight / 2f else 0f

    val netRotation = if (mirror) {
        (sensorOrientation + displayRotationDegrees) % 360
    } else {
        (sensorOrientation - displayRotationDegrees + 360) % 360
    }

    // When net rotation is 90 or 270 degrees, apply aspect scale:
    // X scaled by viewHeight / viewWidth, Y scaled by viewWidth / viewHeight around center before rotating.
    // When net rotation is 0 or 180, scale is 1.0 (no distortion).
    val scaleX = if (netRotation == 90 || netRotation == 270) {
        if (viewWidth > 0 && viewHeight > 0) viewHeight.toFloat() / viewWidth.toFloat() else 1.0f
    } else {
        1.0f
    }
    val scaleY = if (netRotation == 90 || netRotation == 270) {
        if (viewWidth > 0 && viewHeight > 0) viewWidth.toFloat() / viewHeight.toFloat() else 1.0f
    } else {
        1.0f
    }

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
 * Rotates, scales, and mirrors a preview surface to eliminate distortion and black borders.
 *
 * Camera sensors are mounted at an angle (usually 90° for rear, 270° for front) and
 * report frames in their own coordinate space.
 * When net rotation is 90 or 270 degrees, swapping width and height on a non-square
 * view causes aspect distortion (stretching or letterboxing). Applying an aspect scale
 * factor around the center point before rotating ensures the image fills the viewport
 * edge-to-edge with correct aspect ratio.
 */
internal fun previewTransform(
    sensorOrientation: Int,
    displayRotationDegrees: Int = 0,
    mirror: Boolean = false,
    viewWidth: Int,
    viewHeight: Int,
): Matrix {
    val matrix = Matrix()
    if (viewWidth <= 0 || viewHeight <= 0) return matrix

    val calc = calculatePreviewTransform(
        sensorOrientation = sensorOrientation,
        displayRotationDegrees = displayRotationDegrees,
        mirror = mirror,
        viewWidth = viewWidth,
        viewHeight = viewHeight,
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

/**
 * Backward compatibility overload for 4-argument call.
 */
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
