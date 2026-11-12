package com.phonecam.camera

import android.app.Application
import android.os.Build
import com.phonecam.camera.protocol.Log
import com.phonecam.camera.protocol.LogLevel

/**
 * Process-wide setup (PRD §36, §63).
 *
 * Deliberately tiny. The only global state is the diagnostics log; the device
 * identity lives in [com.phonecam.camera.data.IdentityStore] and every session
 * lives in the foreground service. Nothing here opens a camera, a socket, or a
 * notification: those all follow an explicit user action, so PhoneCam never
 * shows a permanent notification that the user did not ask for.
 */
class PhoneCamApplication : Application() {

    override fun onCreate() {
        super.onCreate()
        Log.configure(this, LogLevel.INFO)
        Log.i("PhoneCam ${BuildConfig.VERSION_NAME} starting on ${Build.MANUFACTURER} ${Build.MODEL} (API ${Build.VERSION.SDK_INT})")
        installCrashLogger()
    }

    /**
     * Records an uncaught failure in the diagnostics log before the process dies.
     *
     * The exception is still handed to the platform handler — swallowing it would
     * hide real bugs — but the log file and the diagnostics panel keep the cause,
     * which is what makes a user's bug report actionable.
     */
    private fun installCrashLogger() {
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, error ->
            Log.e(
                "uncaught ${error.javaClass.simpleName} on ${thread.name}: ${error.message}",
                error,
            )
            previous?.uncaughtException(thread, error)
        }
    }
}
