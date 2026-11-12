package com.phonecam.camera.ui.components

import android.graphics.Matrix
import android.graphics.SurfaceTexture
import android.view.Surface
import android.view.TextureView
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
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
 * The display correction is the camera's own [orientationDegrees] (the degrees
 * its sensor is rotated relative to the device's natural orientation), never the
 * desktop's `set_image.rotation`. That rotation is a display transform the
 * desktop applies to *its* output frame (docs/protocol.md §8); it says nothing
 * about how the phone's own panel should show the picture, and applying it here
 * turned the viewfinder to match the PC's chosen output orientation instead of
 * the phone's.
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
) {
    Box(
        modifier = modifier
            .fillMaxWidth()
            .aspectRatio(aspectRatio)
            .clip(PhoneCamShapes.Panel),
    ) {
        AndroidView(
            modifier = Modifier.matchParentSize(),
            factory = { viewContext ->
                TextureView(viewContext).apply {
                    surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                        override fun onSurfaceTextureAvailable(
                            texture: SurfaceTexture,
                            width: Int,
                            height: Int,
                        ) {
                            setTransform(previewTransform(orientationDegrees, mirror, width, height))
                            onSurfaceAvailable(Surface(texture))
                        }

                        override fun onSurfaceTextureSizeChanged(
                            texture: SurfaceTexture,
                            width: Int,
                            height: Int,
                        ) {
                            setTransform(previewTransform(orientationDegrees, mirror, width, height))
                        }

                        override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
                            onSurfaceAvailable(null)
                            return true
                        }

                        override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit
                    }
                }
            },
            update = { view ->
                view.setTransform(previewTransform(orientationDegrees, mirror, view.width, view.height))
            },
        )
    }

    DisposableEffect(Unit) {
        onDispose { onSurfaceAvailable(null) }
    }
}

/**
 * Rotates and mirrors a preview surface to match the phone's orientation.
 *
 * Camera sensors are mounted at an angle and report frames in their own
 * coordinate space, so a viewfinder without this correction appears sideways.
 * The encoder applies its own transform separately; this only affects the display.
 */
internal fun previewTransform(orientationDegrees: Int, mirror: Boolean, viewWidth: Int, viewHeight: Int): Matrix {
    val matrix = Matrix()
    if (viewWidth <= 0 || viewHeight <= 0) return matrix
    val centerX = viewWidth / 2f
    val centerY = viewHeight / 2f
    if (orientationDegrees != 0) {
        matrix.postRotate(orientationDegrees.toFloat(), centerX, centerY)
    }
    if (mirror) {
        matrix.postScale(-1f, 1f, centerX, centerY)
    }
    return matrix
}
