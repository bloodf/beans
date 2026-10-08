package app.beans.core

import android.app.Activity
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import android.net.Uri
import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest
import java.util.UUID

internal class Updater(private val context: Context, private val progress: (Long, Long) -> Unit = { _, _ -> }) {
  private data class Offer(val id: String, val version: String, val code: Long, val size: Long, val hash: String, val notes: String, val readinessHash: String)
  private var offer: Offer? = null
  private var busy = false
  private var session: Int? = null
  private var committed = false
  private var downloadSize = 0L
  private var operation: UpdateOperation? = null
  private val pm get() = context.packageManager
  init {
    pm.packageInstaller.mySessions.forEach { runCatching { pm.packageInstaller.abandonSession(it.sessionId) } }
    File(context.cacheDir, "beans-update.apk").delete()
    context.getSharedPreferences("beans-updates", Context.MODE_PRIVATE).edit().remove("session").apply()
  }
  private fun installed() = pm.getPackageInfo(context.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
  private fun code(info: android.content.pm.PackageInfo) = info.longVersionCode
  fun capability(): String {
    if (context.packageName != "ai.amoena.beans") return "development"
    if (Build.VERSION.SDK_INT < 28) return "native_verifier_unavailable"
    if (pm.getApplicationInfo(context.packageName, PackageManager.GET_META_DATA).metaData?.getBoolean("app.beans.GITHUB_UPDATES", false) != true) return "store_managed"
    val source = if (Build.VERSION.SDK_INT >= 30) pm.getInstallSourceInfo(context.packageName).installingPackageName else pm.getInstallerPackageName(context.packageName)
    return if (source == "com.android.vending") "store_managed" else "supported"
  }
  @Synchronized fun result(): String? {
    val prefs = context.getSharedPreferences("beans-updates", Context.MODE_PRIVATE)
    val result = prefs.getString("result", null)
    if (result != null) { session = null; committed = false; prefs.edit().remove("result").apply() }
    return result
  }
  private fun eligible() { check(capability() == "supported") { "This distribution uses store updates" } }
  private fun version(value: String): List<Long> {
    require(Regex("(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)").matches(value)) { "Invalid version" }
    return value.split('.').map { it.toLong() }
  }
  private fun newer(a: String, b: String): Boolean {
    val x = version(a); val y = version(b)
    for (i in 0..2) if (x[i] != y[i]) return x[i] > y[i]
    return false
  }
  private fun digest(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
  private fun fetch(url: String, limit: Long, output: java.io.OutputStream): Long =
    (operation ?: error("No active update")).fetch(url, limit, output) { count ->
      if (downloadSize > 0) progress(count, downloadSize)
    }
  private fun bytes(url: String, limit: Long): ByteArray {
    val output = java.io.ByteArrayOutputStream()
    fetch(url, limit, output)
    return output.toByteArray()
  }
  private fun asset(v: String, name: String) = "https://github.com/bloodf/beans/releases/download/beans-v$v/$name"
  private fun json(bytes: ByteArray) = UpdateJson.parse(bytes)
  private fun integer(obj: JSONObject, key: String) = UpdateJson.integer(obj, key)
  private fun verified(v: String, notes: String): Offer? {
    val raw = bytes(asset(v, "beans-update.json"), 262144)
    val sigText = bytes(asset(v, "beans-update.json.sig"), 256).toString(Charsets.US_ASCII).trim()
    require(Regex("[A-Za-z0-9+/]{86}==").matches(sigText)) { "Invalid signature encoding" }
    val key = context.assets.open("public-key.txt").use { Base64.decode(it.readBytes().toString(Charsets.US_ASCII).trim(), Base64.NO_WRAP) }
    require(UpdateTrust.verify(raw, Base64.decode(sigText, Base64.NO_WRAP), key)) { "Invalid Beans release signature" }
    val manifest = json(raw)
    require(integer(manifest, "schema") == 1L && manifest.getString("version") == v && Regex("[a-f0-9]{40}").matches(manifest.getString("revision")) && integer(manifest, "protocol") == 5L) { "Unsupported release readiness" }
    val entries = manifest.getJSONArray("artifacts"); require(entries.length() in 1..128)
    val inventory = mutableMapOf<String, JSONObject>()
    for (i in 0 until entries.length()) {
      val entry = entries.getJSONObject(i); val name = entry.getString("name")
      require(Regex("[A-Za-z0-9 ._-]{1,160}").matches(name) && inventory.put(name, entry) == null) { "Invalid or duplicate inventory" }
      require(integer(entry, "size") > 0 && Regex("[a-f0-9]{64}").matches(entry.getString("sha256")))
    }
    val apk = inventory["Beans-$v.apk"] ?: return null
    val proofEntry = inventory["eas-github.json"] ?: error("Missing signed APK provenance")
    require(apk.getString("component") == "android" && apk.getString("platform") == "android" && apk.getString("version") == v && apk.getLong("size") <= 536870912)
    require(proofEntry.getString("component") == "provenance" && proofEntry.getString("platform") == "mobile" && proofEntry.getString("version") == v && proofEntry.getLong("size") <= 65536)
    val proofRaw = bytes(asset(v, "eas-github.json"), proofEntry.getLong("size"))
    require(proofRaw.size.toLong() == proofEntry.getLong("size") && digest(proofRaw) == proofEntry.getString("sha256"))
    val proof = json(proofRaw)
    require(integer(proof, "schema") == 1L && proof.getString("revision") == manifest.getString("revision") && proof.getString("version") == v && proof.getString("profile") == "github" && proof.getString("platform") == "ANDROID" && proof.getString("artifact") == "Beans-$v.apk" && integer(proof, "size") == apk.getLong("size") && proof.getString("sha256") == apk.getString("sha256"))
    UUID.fromString(proof.getString("project")); UUID.fromString(proof.getString("buildId"))
    val build = proof.getString("buildNumber"); require(Regex("[1-9][0-9]*").matches(build))
    val number = build.toLong(); require(number <= 2100000000 && number > code(installed())) { "APK build is not newer" }
    return Offer(UUID.randomUUID().toString(), v, number, apk.getLong("size"), apk.getString("sha256"), notes.take(16000), digest(raw))
  }
  fun check(): Map<String, Any>? {
    synchronized(this) {
      eligible(); check(!busy && session == null) { "Update already running" }
      offer = null; operation = UpdateOperation(120000); busy = true
    }
    try {
    val releases = JSONArray(bytes("https://api.github.com/repos/bloodf/beans/releases?per_page=100&page=1", 1048576).toString(Charsets.UTF_8))
    require(releases.length() <= 100)
    val current = installed().versionName ?: error("Missing installed version")
    val candidates = (0 until releases.length()).map { releases.getJSONObject(it) }.filter {
      !it.getBoolean("draft") && !it.getBoolean("prerelease") && it.getString("tag_name").startsWith("beans-v") &&
        runCatching { newer(it.getString("tag_name").removePrefix("beans-v"), current) }.getOrDefault(false)
    }.sortedWith { a, b -> val x = a.getString("tag_name").removePrefix("beans-v"); val y = b.getString("tag_name").removePrefix("beans-v"); if (x == y) 0 else if (newer(x, y)) -1 else 1 }
    for (candidate in candidates.take(20)) {
      operation!!.ensureActive()
      val names = candidate.getJSONArray("assets")
      val available = (0 until names.length()).map { names.getJSONObject(it).getString("name") }.toSet()
      if (!available.containsAll(listOf("beans-update.json", "beans-update.json.sig"))) continue
      val found = verified(candidate.getString("tag_name").removePrefix("beans-v"), candidate.optString("body")) ?: continue
      synchronized(this) {
        operation!!.ensureActive(); offer = found
        return mapOf("id" to found.id, "version" to found.version, "notes" to found.notes)
      }
    }
    return null
    } finally { synchronized(this) { operation?.close(); operation = null; busy = false } }
  }
  fun cancel() {
    synchronized(this) {
      offer = null
      operation?.cancel()
      if (!committed) { session?.let { runCatching { pm.packageInstaller.abandonSession(it) } }; session = null }
    }
  }
  fun install(id: String, activity: Activity) {
    val selected = synchronized(this) {
      eligible(); check(!busy && session == null) { "Update already running" }
      check(activity.hasWindowFocus()) { "Return to Beans before installing" }
      val value = offer ?: error("Check for updates again")
      require(value.id == id) { "Release consent expired" }; offer = null
      if (!pm.canRequestPackageInstalls()) {
        activity.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}")))
        error("Allow this source, then check and consent again")
      }
      busy = true; operation = UpdateOperation(600000); value
    }
    val file = File(context.cacheDir, "beans-update.apk")
    try {
      val fresh = verified(selected.version, selected.notes) ?: error("Release is no longer Android-ready")
      require(fresh.readinessHash == selected.readinessHash && fresh.hash == selected.hash && fresh.size == selected.size && fresh.code == selected.code) { "Release changed; consent again" }
      downloadSize = selected.size
      file.outputStream().use { require(fetch(asset(selected.version, "Beans-${selected.version}.apk"), selected.size, it) == selected.size) { "Truncated APK" } }
      val hash = MessageDigest.getInstance("SHA-256")
      file.inputStream().use { input -> val buffer = ByteArray(65536); while (true) { operation!!.ensureActive(); val n = input.read(buffer); if (n < 0) break; hash.update(buffer, 0, n) } }
      require(hash.digest().joinToString("") { "%02x".format(it) } == selected.hash) { "APK hash mismatch" }
      val archive = pm.getPackageArchiveInfo(file.path, PackageManager.GET_SIGNING_CERTIFICATES) ?: error("Invalid APK")
      val local = installed()
      require(archive.packageName == context.packageName && archive.versionName == selected.version && code(archive) == selected.code && code(archive) > code(local)) { "APK identity differs" }
      fun signers(info: android.content.pm.PackageInfo) = info.signingInfo?.apkContentsSigners?.map { digest(it.toByteArray()) }?.toSet() ?: emptySet()
      val history = archive.signingInfo?.signingCertificateHistory?.map { digest(it.toByteArray()) }?.toSet() ?: emptySet()
      require(UpdateTrust.compatible(signers(local), signers(archive), history)) { "APK signer incompatible with installed Beans" }
      operation!!.ensureActive()
      check(activity.hasWindowFocus()) { "Installation cancelled in background; consent again" }
      eligible()
      val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
      params.setAppPackageName(context.packageName); params.setSize(selected.size)
      if (Build.VERSION.SDK_INT >= 31) params.setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_REQUIRED)
      val installer = pm.packageInstaller
      val sessionId = installer.createSession(params); synchronized(this) { session = sessionId }
      context.getSharedPreferences("beans-updates", Context.MODE_PRIVATE).edit().putInt("session", sessionId).apply()
      operation!!.ensureActive()
      installer.openSession(sessionId).use { target ->
        target.openWrite("base.apk", 0, selected.size).use { output ->
          file.inputStream().use { input ->
            val buffer = ByteArray(65536)
            while (true) { operation!!.ensureActive(); val n = input.read(buffer); if (n < 0) break; output.write(buffer, 0, n) }
          }
          target.fsync(output)
        }
        val intent = Intent(context, UpdateReceiver::class.java).putExtra("session", sessionId)
        val pending = PendingIntent.getBroadcast(context, sessionId, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE)
        synchronized(this) {
          operation!!.ensureActive()
          target.commit(pending.intentSender)
          committed = true
        }
      }
    } catch (error: Exception) { cancel(); throw error }
    finally { file.delete(); synchronized(this) { operation?.close(); operation = null; busy = false; downloadSize = 0 } }
  }
}
