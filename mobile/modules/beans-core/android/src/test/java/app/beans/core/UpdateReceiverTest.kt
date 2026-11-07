package app.beans.core

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.PackageInstaller
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [32, 33])
class UpdateReceiverTest {
  private val context = RuntimeEnvironment.getApplication()
  private val installer = context.packageManager.packageInstaller
  private val prefs = context.getSharedPreferences("beans-updates", Context.MODE_PRIVATE)

  private fun session(): Int {
    val id = installer.createSession(PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL))
    prefs.edit().clear().putInt("session", id).commit()
    return id
  }

  private fun broadcast(id: Int, confirmation: Intent? = null): Intent = Intent()
    .putExtra("session", id)
    .putExtra(PackageInstaller.EXTRA_SESSION_ID, id)
    .putExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_PENDING_USER_ACTION)
    .apply { if (confirmation != null) putExtra(Intent.EXTRA_INTENT, confirmation) }

  private fun assertFailed(id: Int) {
    assertFalse(prefs.contains("session"))
    assertEquals("Installation cancelled or failed. Check and consent again.", prefs.getString("result", null))
    assertNull(installer.getSessionInfo(id))
  }

  @Test fun successfulLaunchKeepsSession() {
    val id = session()
    val confirmation = Intent("app.beans.TEST_CONFIRMATION")
    UpdateReceiver().onReceive(context, broadcast(id, confirmation))
    val launched = shadowOf(context).nextStartedActivity
    assertEquals(confirmation.action, launched.action)
    assertTrue(launched.flags and Intent.FLAG_ACTIVITY_NEW_TASK != 0)
    assertEquals(id, prefs.getInt("session", -1))
    assertFalse(prefs.contains("result"))
    assertNotNull(installer.getSessionInfo(id))
  }

  @Test fun missingConfirmationFailsAndAbandons() {
    val id = session()
    UpdateReceiver().onReceive(context, broadcast(id))
    assertFailed(id)
  }

  @Test fun launchExceptionsFailAndAbandon() {
    for (failure in listOf(ActivityNotFoundException(), SecurityException())) {
      val id = session()
      val throwingContext = object : ContextWrapper(context) {
        override fun startActivity(intent: Intent) { throw failure }
      }
      UpdateReceiver().onReceive(throwingContext, broadcast(id, Intent("app.beans.TEST_CONFIRMATION")))
      assertFailed(id)
    }
  }

  @Test fun mismatchedSessionsAreInert() {
    val id = session()
    for (intent in listOf(
      broadcast(id + 1, Intent("app.beans.TEST_CONFIRMATION")).putExtra(PackageInstaller.EXTRA_SESSION_ID, id),
      broadcast(id, Intent("app.beans.TEST_CONFIRMATION")).putExtra(PackageInstaller.EXTRA_SESSION_ID, id + 1)
    )) {
      UpdateReceiver().onReceive(context, intent)
      assertEquals(id, prefs.getInt("session", -1))
      assertFalse(prefs.contains("result"))
      assertNotNull(installer.getSessionInfo(id))
      assertNull(shadowOf(context).nextStartedActivity)
    }
  }
}
