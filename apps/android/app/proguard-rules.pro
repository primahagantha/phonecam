# PhoneCam Android release rules.
# kotlinx.serialization keeps generated serializers referenced reflectively.
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.**

-keepclassmembers class com.phonecam.camera.** {
    *** Companion;
}
-keepclasseswithmembers class com.phonecam.camera.** {
    kotlinx.serialization.KSerializer serializer(...);
}
-keep,includedescriptorclasses class com.phonecam.camera.**$$serializer { *; }

# ZXing embedded scanner is reached only through the activity intent filter.
-keep class com.journeyapps.barcodescanner.** { *; }
-keep class com.google.zxing.** { *; }

# Preserve Rust JNI bridge entry points against R8 shrinking/obfuscation.
-keep class com.phonecam.camera.nativecore.** { *; }
