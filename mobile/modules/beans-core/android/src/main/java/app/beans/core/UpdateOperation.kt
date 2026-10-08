package app.beans.core

import java.io.OutputStream
import java.net.HttpURLConnection
import java.net.URI
import java.util.Timer
import java.util.TimerTask

/** One deadline covers metadata, redirects and APK transport, not each read separately. */
internal class UpdateOperation(
  timeoutMillis: Long,
  private val connect: (URI) -> HttpURLConnection = { it.toURL().openConnection() as HttpURLConnection }
) : AutoCloseable {
  private val deadline = System.nanoTime() + timeoutMillis * 1_000_000
  private var stopped = false
  private var active: HttpURLConnection? = null
  private val timer = Timer("Beans update deadline", true)
  init {
    timer.schedule(object : TimerTask() { override fun run() { cancel() } }, timeoutMillis)
  }
  @Synchronized fun ensureActive() {
    check(!stopped && System.nanoTime() < deadline) { "Update cancelled or deadline exceeded; consent again" }
  }
  fun cancel() {
    val connection = synchronized(this) { stopped = true; active.also { active = null } }
    connection?.disconnect()
  }
  override fun close() { timer.cancel(); cancel() }
  fun fetch(url: String, limit: Long, output: OutputStream, progress: (Long) -> Unit = {}): Long {
    var uri = URI(url)
    val seen = mutableSetOf<String>()
    repeat(6) {
      ensureActive()
      require(uri.scheme == "https" && uri.userInfo == null && uri.fragment == null && (uri.port == -1 || uri.port == 443)) { "Invalid update transport" }
      require((uri.host == "api.github.com" && uri.path == "/repos/bloodf/beans/releases") ||
        (uri.host == "github.com" && uri.path.startsWith("/bloodf/beans/releases/download/beans-v")) ||
        (uri.host == "release-assets.githubusercontent.com" && uri.path.startsWith("/github-production-release-asset/"))) { "Untrusted update host or path" }
      require(seen.add(uri.toString())) { "Redirect loop" }
      val connection = connect(uri)
      try {
        synchronized(this) { ensureActive(); active = connection }
        connection.instanceFollowRedirects = false
        connection.connectTimeout = 15000; connection.readTimeout = 30000
        connection.setRequestProperty("Accept", "application/octet-stream")
        connection.setRequestProperty("User-Agent", "Beans-Android-Updater")
        ensureActive()
        val status = connection.responseCode
        ensureActive()
        if (status == 301 || status == 302 || status == 303 || status == 307 || status == 308) {
          uri = uri.resolve(connection.getHeaderField("Location") ?: error("Missing redirect"))
        } else {
          check(status == 200) { "Update request failed ($status)" }
          require(connection.contentLengthLong <= limit) { "Update exceeds size limit" }
          var count = 0L
          connection.inputStream.use { input ->
            val buffer = ByteArray(65536)
            while (true) {
              ensureActive()
              val n = input.read(buffer)
              ensureActive()
              if (n < 0) break
              count += n; require(count <= limit) { "Update exceeds size limit" }
              output.write(buffer, 0, n)
              progress(count)
            }
          }
          ensureActive()
          return count
        }
      } finally {
        synchronized(this) { if (active === connection) active = null }
        connection.disconnect()
      }
    }
    error("Too many redirects")
  }
}
