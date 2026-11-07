package app.beans.core

import org.junit.Assert.*
import org.junit.Test

class UpdateDistributionTest {
  @Test fun sideloadAndOwnUpdatesOnly() {
    assertTrue(UpdateDistribution.allowsInstaller(null, "ai.amoena.beans"))
    assertTrue(UpdateDistribution.allowsInstaller("ai.amoena.beans", "ai.amoena.beans"))
    for (source in listOf("com.android.vending", "com.amazon.venezia", "org.fdroid.fdroid", "com.android.packageinstaller", "", "ai.amoena.beans.dev")) {
      assertFalse(source, UpdateDistribution.allowsInstaller(source, "ai.amoena.beans"))
    }
  }
}
