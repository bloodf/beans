package app.beans.core

import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.ServerSocket
import java.net.URI
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class UpdateOperationTest {
  private val url = "https://api.github.com/repos/bloodf/beans/releases"

  @Test fun cancellationDisconnectsBlockedMetadataAndAllowsFreshOperation() {
    ServerSocket(0).use { server ->
      val received = CountDownLatch(1)
      val release = CountDownLatch(1)
      val pool = Executors.newFixedThreadPool(2)
      try {
        val serving = pool.submit {
          server.accept().use { socket ->
            val reader = socket.getInputStream().bufferedReader()
            while (!reader.readLine().isNullOrEmpty()) { }
            socket.getOutputStream().write("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n".toByteArray())
            socket.getOutputStream().flush()
            received.countDown()
            release.await(3, TimeUnit.SECONDS)
          }
        }
        UpdateOperation(5000) { URI("http://127.0.0.1:${server.localPort}/").toURL().openConnection() as HttpURLConnection }.use { operation ->
          val output = ByteArrayOutputStream()
          val request = pool.submit<Boolean> { try { operation.fetch(url, 100, output); false } catch (_: Exception) { true } }
          assertTrue(received.await(2, TimeUnit.SECONDS))
          operation.cancel()
          assertTrue(request.get(2, TimeUnit.SECONDS))
          assertEquals(0, output.size())
        }
        release.countDown(); serving.get(2, TimeUnit.SECONDS)
        UpdateOperation(5000).use { it.ensureActive() }
      } finally { release.countDown(); pool.shutdownNow() }
    }
  }

  @Test fun totalDeadlineStopsSlowChunksAndPreventsFollowingRequest() {
    var connections = 0
    val stopped = CountDownLatch(1)
    val output = ByteArrayOutputStream()
    UpdateOperation(150) { uri ->
      connections++
      object : HttpURLConnection(uri.toURL()) {
        override fun connect() { }
        override fun usingProxy() = false
        override fun disconnect() { stopped.countDown() }
        override fun getResponseCode() = 200
        override fun getContentLengthLong() = -1L
        override fun getInputStream() = object : java.io.InputStream() {
          override fun read(): Int { stopped.await(20, TimeUnit.MILLISECONDS); return 1 }
          override fun read(buffer: ByteArray, offset: Int, length: Int): Int { buffer[offset] = read().toByte(); return 1 }
        }
      }
    }.use { operation ->
      try { operation.fetch(url, 100000, output); fail("Slow metadata exceeded deadline") } catch (_: IllegalStateException) { }
      assertEquals(0L, stopped.count)
      val count = output.size()
      try { operation.fetch(url, 100, output); fail("Cancelled revalidation started APK request") } catch (_: IllegalStateException) { }
      assertEquals(1, connections)
      assertEquals(count, output.size())
    }
  }
}
