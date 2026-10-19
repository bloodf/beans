package app.beans.core

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build

class UpdateReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) {
    val expected = context.getSharedPreferences("beans-updates", Context.MODE_PRIVATE).getInt("session", -1)
    if (expected < 0 || intent.getIntExtra("session", -2) != expected || intent.getIntExtra(PackageInstaller.EXTRA_SESSION_ID, -3) != expected) return
    val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
    if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
      val confirmation = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java) else @Suppress("DEPRECATION") intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)
      if (confirmation != null) {
        context.startActivity(confirmation.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        return
      }
    }
    context.getSharedPreferences("beans-updates", Context.MODE_PRIVATE).edit().remove("session").putString("result", if (status == PackageInstaller.STATUS_SUCCESS) "installed" else "Installation cancelled or failed. Check and consent again.").apply()
    if (status != PackageInstaller.STATUS_SUCCESS) runCatching { context.packageManager.packageInstaller.abandonSession(expected) }
  }
}
