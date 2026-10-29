package com.gamehopper.launcher.hopper

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class PathArgs {
    lateinit var path: String
}

@InvokeArg
class PackageArgs {
    lateinit var packageName: String
}

@InvokeArg
class UrlArgs {
    lateinit var url: String
}

@TauriPlugin
class HopperPlugin(private val activity: Activity) : Plugin(activity) {

    /** The package name written inside a downloaded APK. */
    @Command
    fun apkPackageName(invoke: Invoke) {
        val args = invoke.parseArgs(PathArgs::class.java)
        val info = activity.packageManager.getPackageArchiveInfo(args.path, 0)
        if (info == null) {
            invoke.reject("That file is not a valid Android app")
            return
        }
        val ret = JSObject()
        ret.put("packageName", info.packageName)
        invoke.resolve(ret)
    }

    /** Opens Android's own installer for a downloaded APK. */
    @Command
    fun installApk(invoke: Invoke) {
        val args = invoke.parseArgs(PathArgs::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && !activity.packageManager.canRequestPackageInstalls()) {
            // Android asks once per app: send the user to the switch, then they tap Install again.
            val settings = Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + activity.packageName))
            settings.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            activity.startActivity(settings)
            invoke.reject("Allow installs from Hopper Launcher in the screen that just opened, then tap Install again.")
            return
        }
        val uri = FileProvider.getUriForFile(activity, activity.packageName + ".hopper.fileprovider", File(args.path))
        val intent = Intent(Intent.ACTION_VIEW)
        intent.setDataAndType(uri, "application/vnd.android.package-archive")
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        activity.startActivity(intent)
        invoke.resolve()
    }

    @Command
    fun isInstalled(invoke: Invoke) {
        val args = invoke.parseArgs(PackageArgs::class.java)
        val installed = try {
            activity.packageManager.getPackageInfo(args.packageName, 0)
            true
        } catch (e: PackageManager.NameNotFoundException) {
            false
        }
        val ret = JSObject()
        ret.put("installed", installed)
        invoke.resolve(ret)
    }

    @Command
    fun launchPackage(invoke: Invoke) {
        val args = invoke.parseArgs(PackageArgs::class.java)
        val intent = activity.packageManager.getLaunchIntentForPackage(args.packageName)
        if (intent == null) {
            invoke.reject("That app is not installed or cannot be opened")
            return
        }
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        activity.startActivity(intent)
        invoke.resolve()
    }

    /** Opens Android's own uninstall confirmation. */
    @Command
    fun uninstallPackage(invoke: Invoke) {
        val args = invoke.parseArgs(PackageArgs::class.java)
        val intent = Intent(Intent.ACTION_DELETE, Uri.parse("package:" + args.packageName))
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        activity.startActivity(intent)
        invoke.resolve()
    }

    @Command
    fun openUrl(invoke: Invoke) {
        val args = invoke.parseArgs(UrlArgs::class.java)
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse(args.url))
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        activity.startActivity(intent)
        invoke.resolve()
    }
}
