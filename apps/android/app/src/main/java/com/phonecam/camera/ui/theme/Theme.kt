package com.phonecam.camera.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.colorResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.phonecam.camera.R

/**
 * Semantic studio colour tokens that sit alongside Material3's [ColorScheme].
 *
 * Separating live status green and subtle danger surfaces from the Studio Blue
 * primary accent ensures destructive actions (Stop PhoneCam, Forget PC) never
 * borrow green or primary button styling, and live status indicators carry a
 * single unambiguous visual meaning.
 */
@Immutable
data class StudioColors(
    val stageBackground: Color,
    val live: Color,
    val liveContainer: Color,
    val onLiveContainer: Color,
    val dangerSurface: Color,
    val dangerBorder: Color,
    val dangerText: Color,
)

private val DefaultDarkStudioColors = StudioColors(
    stageBackground = Color(0xFF06080C),
    live = Color(0xFF22C55E),
    liveContainer = Color(0xFF0E291B),
    onLiveContainer = Color(0xFF86EFAC),
    dangerSurface = Color(0xFF2A1418),
    dangerBorder = Color(0xFF7F1D1D),
    dangerText = Color(0xFFFCA5A5),
)

private val LocalStudioColors = staticCompositionLocalOf { DefaultDarkStudioColors }

/**
 * Accessor for PhoneCam semantic studio tokens inside any [PhoneCamTheme] scope.
 */
object PhoneCamStudioTheme {
    val colors: StudioColors
        @Composable
        get() = LocalStudioColors.current
}

/**
 * The PhoneCam Studio theme.
 *
 * Fixed Obsidian Dark Studio colours aligned with PhoneCam Desktop GPUI, with a
 * clean daylight counterpart when the system uses light mode. Every colour lives
 * in `res/values/colors.xml` so the launch window and Compose UI cannot drift.
 */
@Composable
fun PhoneCamTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    val colorScheme = if (darkTheme) phoneCamDarkColors() else phoneCamLightColors()
    val studioColors = if (darkTheme) phoneCamDarkStudioColors() else phoneCamLightStudioColors()

    CompositionLocalProvider(LocalStudioColors provides studioColors) {
        MaterialTheme(
            colorScheme = colorScheme,
            typography = PhoneCamTypography,
            shapes = MaterialShapes,
            content = content,
        )
    }
}

/**
 * The corner-radius scale — deliberate hierarchy, never uniform pills everywhere:
 *
 *  - [Stage]: the live camera monitor container (14dp).
 *  - [Panel]: cards, banners, and top-level containers (12dp).
 *  - [Field]: inputs, recessed wells, and standard studio action buttons (10dp).
 *  - [Chip]: compact controls, tokens, and viewfinder quick actions (8dp).
 *  - [Control]: status pills only, reserving capsule geometry for live readouts.
 */
object PhoneCamShapes {
    val Stage = RoundedCornerShape(14.dp)
    val Panel = RoundedCornerShape(12.dp)
    val Field = RoundedCornerShape(10.dp)
    val Chip = RoundedCornerShape(8.dp)
    val Control = RoundedCornerShape(percent = 50)
}

private val MaterialShapes = Shapes(
    extraSmall = PhoneCamShapes.Chip,
    small = PhoneCamShapes.Field,
    medium = PhoneCamShapes.Panel,
    large = PhoneCamShapes.Panel,
    extraLarge = PhoneCamShapes.Stage,
)

private val PhoneCamTypography = Typography().let { base ->
    base.copy(
        titleMedium = base.titleMedium.copy(
            fontSize = 16.sp,
            lineHeight = 22.sp,
            fontWeight = FontWeight.SemiBold,
            letterSpacing = (-0.15).sp,
        ),
        titleSmall = base.titleSmall.copy(
            fontSize = 14.sp,
            lineHeight = 20.sp,
            fontWeight = FontWeight.SemiBold,
            letterSpacing = 0.sp,
        ),
        bodyMedium = base.bodyMedium.copy(
            fontSize = 14.sp,
            lineHeight = 20.sp,
        ),
        labelLarge = base.labelLarge.copy(
            fontSize = 13.sp,
            lineHeight = 18.sp,
            fontWeight = FontWeight.SemiBold,
            letterSpacing = 0.1.sp,
        ),
        labelMedium = base.labelMedium.copy(
            fontSize = 12.sp,
            lineHeight = 16.sp,
            fontWeight = FontWeight.Medium,
            letterSpacing = 0.15.sp,
        ),
        labelSmall = base.labelSmall.copy(
            fontSize = 11.sp,
            lineHeight = 14.sp,
            fontWeight = FontWeight.Medium,
            letterSpacing = 0.2.sp,
        ),
        bodySmall = base.bodySmall.copy(
            fontSize = 12.sp,
            lineHeight = 17.sp,
        ),
    )
}

/** The warning role, mapped onto Material's `tertiary` slot. */
val ColorScheme.warning: Color get() = tertiary
val ColorScheme.warningContainer: Color get() = tertiaryContainer
val ColorScheme.onWarningContainer: Color get() = onTertiaryContainer

/** The info role, mapped onto Material's `secondary` slot. */
val ColorScheme.infoContainer: Color get() = secondaryContainer
val ColorScheme.onInfoContainer: Color get() = onSecondaryContainer

@Composable
private fun phoneCamDarkStudioColors() = StudioColors(
    stageBackground = colorResource(R.color.phonecam_stage),
    live = colorResource(R.color.phonecam_live),
    liveContainer = colorResource(R.color.phonecam_live_container),
    onLiveContainer = colorResource(R.color.phonecam_on_live_container),
    dangerSurface = colorResource(R.color.phonecam_error_container),
    dangerBorder = colorResource(R.color.phonecam_danger_border),
    dangerText = colorResource(R.color.phonecam_on_error_container),
)

@Composable
private fun phoneCamLightStudioColors() = StudioColors(
    stageBackground = colorResource(R.color.phonecam_stage),
    live = colorResource(R.color.phonecam_light_live),
    liveContainer = colorResource(R.color.phonecam_light_live_container),
    onLiveContainer = colorResource(R.color.phonecam_light_on_live_container),
    dangerSurface = colorResource(R.color.phonecam_light_error_container),
    dangerBorder = colorResource(R.color.phonecam_light_danger_border),
    dangerText = colorResource(R.color.phonecam_light_on_error_container),
)

@Composable
private fun phoneCamDarkColors() = darkColorScheme(
    primary = colorResource(R.color.phonecam_primary),
    onPrimary = colorResource(R.color.phonecam_on_primary),
    primaryContainer = colorResource(R.color.phonecam_primary_container),
    onPrimaryContainer = colorResource(R.color.phonecam_on_primary_container),

    background = colorResource(R.color.phonecam_background),
    onBackground = colorResource(R.color.phonecam_on_background),
    surface = colorResource(R.color.phonecam_surface),
    onSurface = colorResource(R.color.phonecam_on_surface),
    surfaceVariant = colorResource(R.color.phonecam_surface_variant),
    onSurfaceVariant = colorResource(R.color.phonecam_on_surface_variant),

    surfaceContainerLowest = colorResource(R.color.phonecam_background),
    surfaceContainerLow = colorResource(R.color.phonecam_surface),
    surfaceContainer = colorResource(R.color.phonecam_surface),
    surfaceContainerHigh = colorResource(R.color.phonecam_surface_variant),
    surfaceContainerHighest = colorResource(R.color.phonecam_surface_variant),

    outline = colorResource(R.color.phonecam_outline),
    outlineVariant = colorResource(R.color.phonecam_outline_variant),
    surfaceTint = Color.Transparent,

    secondary = colorResource(R.color.phonecam_info_container),
    onSecondary = colorResource(R.color.phonecam_on_info_container),
    secondaryContainer = colorResource(R.color.phonecam_info_container),
    onSecondaryContainer = colorResource(R.color.phonecam_on_info_container),

    tertiary = colorResource(R.color.phonecam_warning),
    onTertiary = colorResource(R.color.phonecam_warning_container),
    tertiaryContainer = colorResource(R.color.phonecam_warning_container),
    onTertiaryContainer = colorResource(R.color.phonecam_on_warning_container),

    error = colorResource(R.color.phonecam_error),
    onError = colorResource(R.color.phonecam_on_error),
    errorContainer = colorResource(R.color.phonecam_error_container),
    onErrorContainer = colorResource(R.color.phonecam_on_error_container),

    inverseSurface = colorResource(R.color.phonecam_inverse_surface),
    inverseOnSurface = colorResource(R.color.phonecam_inverse_on_surface),
    inversePrimary = colorResource(R.color.phonecam_inverse_primary),
)

@Composable
private fun phoneCamLightColors() = lightColorScheme(
    primary = colorResource(R.color.phonecam_light_primary),
    onPrimary = colorResource(R.color.phonecam_light_on_primary),
    primaryContainer = colorResource(R.color.phonecam_light_primary_container),
    onPrimaryContainer = colorResource(R.color.phonecam_light_on_primary_container),

    background = colorResource(R.color.phonecam_light_background),
    onBackground = colorResource(R.color.phonecam_light_on_background),
    surface = colorResource(R.color.phonecam_light_surface),
    onSurface = colorResource(R.color.phonecam_light_on_surface),
    surfaceVariant = colorResource(R.color.phonecam_light_surface_variant),
    onSurfaceVariant = colorResource(R.color.phonecam_light_on_surface_variant),

    surfaceContainerLowest = colorResource(R.color.phonecam_light_background),
    surfaceContainerLow = colorResource(R.color.phonecam_light_surface),
    surfaceContainer = colorResource(R.color.phonecam_light_surface),
    surfaceContainerHigh = colorResource(R.color.phonecam_light_surface_variant),
    surfaceContainerHighest = colorResource(R.color.phonecam_light_surface_variant),

    outline = colorResource(R.color.phonecam_light_outline),
    outlineVariant = colorResource(R.color.phonecam_light_outline_variant),
    surfaceTint = Color.Transparent,

    secondary = colorResource(R.color.phonecam_light_info_container),
    onSecondary = colorResource(R.color.phonecam_light_on_info_container),
    secondaryContainer = colorResource(R.color.phonecam_light_info_container),
    onSecondaryContainer = colorResource(R.color.phonecam_light_on_info_container),

    tertiary = colorResource(R.color.phonecam_light_warning),
    onTertiary = colorResource(R.color.phonecam_light_warning_container),
    tertiaryContainer = colorResource(R.color.phonecam_light_warning_container),
    onTertiaryContainer = colorResource(R.color.phonecam_light_on_warning_container),

    error = colorResource(R.color.phonecam_light_error),
    onError = colorResource(R.color.phonecam_light_on_error),
    errorContainer = colorResource(R.color.phonecam_light_error_container),
    onErrorContainer = colorResource(R.color.phonecam_light_on_error_container),

    inverseSurface = colorResource(R.color.phonecam_light_inverse_surface),
    inverseOnSurface = colorResource(R.color.phonecam_light_inverse_on_surface),
    inversePrimary = colorResource(R.color.phonecam_light_inverse_primary),
)
