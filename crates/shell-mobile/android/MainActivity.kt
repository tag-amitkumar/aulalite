package dev.dioxus.main

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ContentProvider
import android.content.ContentValues
import android.content.Intent
import android.content.pm.PackageManager
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.util.Log
import android.webkit.MimeTypeMap
import androidx.annotation.Keep
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import guru.elementors.aulalite.R
import java.io.File
import java.io.FileNotFoundException
import java.lang.ref.WeakReference
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

typealias BuildConfig = guru.elementors.aulalite.BuildConfig

private const val LOG_TAG = "AulaLiteHost"
private const val NOTIFICATION_CHANNEL_ID = "aulalite_updates"
private const val NOTIFICATION_PERMISSION_REQUEST = 7341
private const val ROUTE_EXTRA = "AULALITE_ROUTE"

class MainActivity : WryActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        AndroidHost.attach(this)
        AndroidHost.ensureFirebase(this)
        AndroidHost.forwardIntent(intent)
        super.onCreate(savedInstanceState)
        AndroidHost.refreshPushToken()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        AndroidHost.forwardIntent(intent)
    }

    fun requestPushPermissionFromRust() {
        runOnUiThread {
            AndroidHost.ensureFirebase(this)
            if (
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) !=
                    PackageManager.PERMISSION_GRANTED
            ) {
                requestPermissions(
                    arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                    NOTIFICATION_PERMISSION_REQUEST,
                )
            }
            AndroidHost.refreshPushToken()
        }
    }

    fun deletePushTokenFromRust() {
        runOnUiThread {
            getSharedPreferences("aulalite_host", 0)
                .edit().remove("pending_fcm_token").apply()
            if (AndroidHost.ensureFirebase(this)) {
                FirebaseMessaging.getInstance().deleteToken()
                    .addOnFailureListener { error ->
                        Log.e(LOG_TAG, "Could not delete the FCM token", error)
                    }
            }
        }
    }

    fun shareFileFromRust(path: String, mimeType: String): Boolean {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            return sharePrivateExport(path, mimeType)
        }
        var result = false
        val completed = CountDownLatch(1)
        runOnUiThread {
            result = sharePrivateExport(path, mimeType)
            completed.countDown()
        }
        return completed.await(4, TimeUnit.SECONDS) && result
    }

    private fun sharePrivateExport(path: String, mimeType: String): Boolean {
        return try {
            val file = File(path).canonicalFile
            val root = File(filesDir, "data/Exports").canonicalFile
            if (!file.isFile || file.parentFile != root) {
                Log.w(LOG_TAG, "Rejected export path outside the private export directory")
                return false
            }
            val uri = Uri.Builder()
                .scheme("content")
                .authority("$packageName.exports")
                .appendPath("export")
                .appendPath(file.name)
                .build()
            val send = Intent(Intent.ACTION_SEND).apply {
                type = mimeType.ifBlank { "application/octet-stream" }
                putExtra(Intent.EXTRA_STREAM, uri)
                clipData = android.content.ClipData.newRawUri(file.name, uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            startActivity(Intent.createChooser(send, "Share AulaLite export"))
            true
        } catch (error: Exception) {
            Log.e(LOG_TAG, "Could not share AulaLite export", error)
            false
        }
    }
}

@Keep
object AndroidHost {
    init {
        System.loadLibrary("main")
    }

    private var activity = WeakReference<MainActivity>(null)

    @JvmStatic private external fun nativeFirebaseApiKey(): String
    @JvmStatic private external fun nativeFirebaseProjectId(): String
    @JvmStatic private external fun nativeFirebaseSenderId(): String
    @JvmStatic private external fun nativeFirebaseAppId(): String
    @JvmStatic private external fun nativePushToken(token: String): Boolean
    @JvmStatic private external fun nativeDeepLink(url: String): Boolean
    @JvmStatic private external fun nativeNotificationRoute(route: String): Boolean

    fun attach(current: MainActivity) {
        activity = WeakReference(current)
    }

    fun ensureFirebase(context: android.content.Context): Boolean {
        if (FirebaseApp.getApps(context).isNotEmpty()) return true
        return try {
            val apiKey = nativeFirebaseApiKey().trim()
            val projectId = nativeFirebaseProjectId().trim()
            val senderId = nativeFirebaseSenderId().trim()
            val appId = nativeFirebaseAppId().trim()
            if (apiKey.isEmpty() || projectId.isEmpty() || senderId.isEmpty() || appId.isEmpty()) {
                Log.e(LOG_TAG, "Android Firebase publishable configuration is incomplete")
                false
            } else {
                val options = FirebaseOptions.Builder()
                    .setApiKey(apiKey)
                    .setProjectId(projectId)
                    .setGcmSenderId(senderId)
                    .setApplicationId(appId)
                    .build()
                FirebaseApp.initializeApp(context, options) != null
            }
        } catch (error: Exception) {
            Log.e(LOG_TAG, "Could not initialize Firebase", error)
            false
        }
    }

    fun refreshPushToken() {
        val current = activity.get() ?: return
        if (!ensureFirebase(current)) return
        val pending = current.getSharedPreferences("aulalite_host", 0)
            .getString("pending_fcm_token", null)
        if (pending != null && nativePushToken(pending)) {
            current.getSharedPreferences("aulalite_host", 0)
                .edit().remove("pending_fcm_token").apply()
        }
        FirebaseMessaging.getInstance().token
            .addOnSuccessListener { token ->
                if (!nativePushToken(token)) Log.w(LOG_TAG, "Rust rejected the FCM token")
            }
            .addOnFailureListener { error -> Log.e(LOG_TAG, "Could not obtain FCM token", error) }
    }

    fun forwardIntent(intent: Intent?) {
        val current = intent ?: return
        current.dataString?.let { url ->
            if (!nativeDeepLink(url)) Log.w(LOG_TAG, "Rust rejected the Android deep link")
        }
        val route = sequenceOf(
            current.getStringExtra(ROUTE_EXTRA),
            current.getStringExtra("route"),
            current.getStringExtra("url"),
        ).firstOrNull { !it.isNullOrBlank() }
        if (route != null && !nativeNotificationRoute(route)) {
            Log.w(LOG_TAG, "Rust rejected the notification route")
        }
    }
}

class AulaLiteMessagingService : FirebaseMessagingService() {
    override fun onCreate() {
        super.onCreate()
        AndroidHost.ensureFirebase(this)
    }

    override fun onNewToken(token: String) {
        super.onNewToken(token)
        // A token can rotate while no Activity (and therefore no initialized
        // ndk-context/keyring) exists. Persist this non-secret provider token
        // privately and move it into Rust secure storage on the next Activity.
        getSharedPreferences("aulalite_host", 0)
            .edit().putString("pending_fcm_token", token).apply()
    }

    override fun onMessageReceived(message: RemoteMessage) {
        super.onMessageReceived(message)
        val title = message.notification?.title ?: message.data["title"] ?: "AulaLite"
        val body = message.notification?.body ?: message.data["body"] ?: "You have a new update."
        val route = message.data["route"] ?: message.data["url"] ?: "/notifications"
        showNotification(title.take(120), body.take(500), route.take(2048))
    }

    private fun showNotification(title: String, body: String, route: String) {
        if (
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) !=
                PackageManager.PERMISSION_GRANTED
        ) return

        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                NotificationChannel(
                    NOTIFICATION_CHANNEL_ID,
                    "AulaLite updates",
                    NotificationManager.IMPORTANCE_DEFAULT,
                ),
            )
        }
        val launch = Intent(this, MainActivity::class.java).apply {
            action = "guru.elementors.aulalite.OPEN_NOTIFICATION"
            putExtra(ROUTE_EXTRA, route)
            flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
        }
        val pending = PendingIntent.getActivity(
            this,
            route.hashCode(),
            launch,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = NotificationCompat.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setAutoCancel(true)
            .setContentIntent(pending)
            .build()
        manager.notify((System.currentTimeMillis() and 0x7fffffff).toInt(), notification)
    }
}

/**
 * Minimal read-only provider for app-private exports. It deliberately exposes
 * one directory and one operation, avoiding broad filesystem path metadata.
 */
class AulaLiteExportProvider : ContentProvider() {
    override fun onCreate(): Boolean = true

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (mode != "r") throw FileNotFoundException("AulaLite exports are read-only")
        val file = resolve(uri)
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun getType(uri: Uri): String {
        val extension = MimeTypeMap.getFileExtensionFromUrl(resolve(uri).name)
        return MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension.lowercase())
            ?: "application/octet-stream"
    }

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor {
        val file = resolve(uri)
        val requested = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        val supported = requested.filter {
            it == OpenableColumns.DISPLAY_NAME || it == OpenableColumns.SIZE
        }
        return MatrixCursor(supported.toTypedArray(), 1).apply {
            addRow(supported.map { column ->
                if (column == OpenableColumns.DISPLAY_NAME) file.name else file.length()
            })
        }
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? =
        throw UnsupportedOperationException("AulaLite exports are read-only")

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ): Int = throw UnsupportedOperationException("AulaLite exports are read-only")

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int =
        throw UnsupportedOperationException("AulaLite exports are read-only")

    private fun resolve(uri: Uri): File {
        val current = context ?: throw FileNotFoundException("Provider context unavailable")
        if (uri.authority != "${current.packageName}.exports" || uri.pathSegments.size != 2 ||
            uri.pathSegments[0] != "export") {
            throw FileNotFoundException("Invalid AulaLite export URI")
        }
        val root = File(current.filesDir, "data/Exports").canonicalFile
        val file = File(root, uri.pathSegments[1]).canonicalFile
        if (file.parentFile != root || !file.isFile) {
            throw FileNotFoundException("AulaLite export not found")
        }
        return file
    }
}
