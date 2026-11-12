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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.colorResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.phonecam.camera.R

/**
 * The PhoneCam theme.
 *
 * Fixed colours rather than Material You dynamic colour: this is a utility that
 * is usually glanced at while the user is looking at their PC, so the palette has
 * to read the same on every phone, and the dark scheme has to be dark because the
 * phone is often the only light source in the room.
 *
 * Every colour is declared in `res/values/colors.xml`, so the XML window theme
 * (which paints the first frame before Compose runs) and this theme cannot drift
 * apart. The contrast of every pair the UI actually renders was checked against
 * WCAG AA (4.5:1 text, 3:1 borders, icons and indicator dots) when the palette
 * was set; see the notes in `colors.xml` for which pair covers which role.
 */
@Composable
fun PhoneCamTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = if (isSystemInDarkTheme()) phoneCamDarkColors() else phoneCamLightColors(),
        typography = PhoneCamTypography,
        shapes = MaterialShapes,
        content = content,
    )
}

/**
 * The corner-radius scale.
 *
 * Three values and a rule for each, so no component invents a fourth:
 *
 *  - [Panel]: every container. Cards, banners, the log well. 12dp, not
 *    Material's 16dp default: at this density a 16dp card reads as a bubble, and
 *    a screen of them turns into a stack of rounded tiles. The panel is a
 *    surface with an edge, not a toy.
 *  - [Field]: every input. Text fields.
 *  - [Control]: every tappable. Buttons, chips, icon buttons, status pills.
 *
 * Anything that does not fit one of those three is a bug in the component, not a
 * reason for a new radius.
 */
object PhoneCamShapes {
    val Panel = RoundedCornerShape(12.dp)
    val Field = RoundedCornerShape(10.dp)
    val Control = RoundedCornerShape(percent = 50)
}

/**
 * Material's shape slots, pointed at the scale above.
 *
 * Text fields read `extraSmall`, cards read `medium`, snackbars read
 * `extraSmall`; mapping them here is what keeps a stock Material component on the
 * same radius as the hand-built ones.
 */
private val MaterialShapes = Shapes(
    extraSmall = PhoneCamShapes.Field,
    small = PhoneCamShapes.Field,
    medium = PhoneCamShapes.Panel,
    large = PhoneCamShapes.Panel,
    extraLarge = PhoneCamShapes.Panel,
)

/**
 * The type scale, restricted to the roles this screen uses.
 *
 * Only the sizes, weights and tracking are changed from Material's defaults: the
 * system font is the right one for a utility that has to be legible at arm's
 * length in a dark room, and shipping a display face for a settings panel would
 * be decoration. What the defaults get wrong for this app is scale: the previous
 * scale hovered between 12sp and 16sp, so a section title and a row value were
 * almost the same size and every screen read as one flat field of text. Body text
 * steps up to 15sp, section titles to 17sp, and the tracking tightens so short
 * technical strings such as "1080p30" stay compact.
 */
private val PhoneCamTypography = Typography().let { base ->
    base.copy(
        titleMedium = base.titleMedium.copy(
            fontSize = 17.sp,
            fontWeight = FontWeight.SemiBold,
            letterSpacing = 0.sp,
        ),
        titleSmall = base.titleSmall.copy(
            fontSize = 15.sp,
            fontWeight = FontWeight.SemiBold,
            letterSpacing = 0.sp,
        ),
        bodyMedium = base.bodyMedium.copy(
            fontSize = 15.sp,
            lineHeight = 22.sp,
        ),
        labelLarge = base.labelLarge.copy(
            fontWeight = FontWeight.SemiBold,
            letterSpacing = 0.1.sp,
        ),
        bodySmall = base.bodySmall.copy(
            fontSize = 13.sp,
            lineHeight = 18.sp,
        ),
    )
}

/**
 * The warning role, on Material's `tertiary` slot.
 *
 * Material 3 has no warning or info role, and its documentation points at
 * tertiary for "contrasting accent" and secondary for "less prominent accent",
 * which is exactly what these two are. Naming them here means the call site reads
 * as intent ("warning container") instead of as a slot number, and the palette
 * file still carries the words `warning` and `info` next to the hex values.
 */
val ColorScheme.warning: Color get() = tertiary
val ColorScheme.warningContainer: Color get() = tertiaryContainer
val ColorScheme.onWarningContainer: Color get() = onTertiaryContainer

/** The info role, on Material's `secondary` slot. */
val ColorScheme.infoContainer: Color get() = secondaryContainer
val ColorScheme.onInfoContainer: Color get() = onSecondaryContainer

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

    // Flat panels: every card is the same tone, separated by a hairline rather
    // than by a shadow, so the container roles collapse onto the two surfaces.
    surfaceContainerLowest = colorResource(R.color.phonecam_background),
    surfaceContainerLow = colorResource(R.color.phonecam_surface),
    surfaceContainer = colorResource(R.color.phonecam_surface),
    surfaceContainerHigh = colorResource(R.color.phonecam_surface_variant),
    surfaceContainerHighest = colorResource(R.color.phonecam_surface_variant),

    outline = colorResource(R.color.phonecam_outline),
    outlineVariant = colorResource(R.color.phonecam_outline_variant),

    // `surfaceTint` is what Material mixes into an elevated surface. Its baseline
    // value is the Material purple, and any component that reaches for it would
    // paint a second accent into an otherwise single-accent app. Nothing here
    // currently elevates, so this is insurance rather than a fix, but a
    // purple-tinted card appearing later would be a real colour-lock break.
    surfaceTint = colorResource(R.color.phonecam_primary),

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

/**
 * The light scheme exists so a phone in daylight is still readable. It keeps the
 * same roles and the same accent hue, taken down in lightness because the brand
 * green is unreadable as text on white.
 */
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

    surfaceTint = colorResource(R.color.phonecam_light_primary),

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
