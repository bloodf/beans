package app.beans.core

import org.junit.Assert.*
import org.junit.Test

class UpdateJsonTest {
  @Test fun rejectsAmbiguousObjects() {
    for (text in listOf("{\"size\":1,\"size\":2}", "{\"artifacts\":[{\"name\":\"a\",\"name\":\"b\"}]}", "{\"size\":1} trailing")) {
      try { UpdateJson.parse(text.toByteArray()); fail("Accepted ambiguous JSON") } catch (_: IllegalArgumentException) { }
    }
  }
  @Test fun numericAuthorityCannotBeCoerced() {
    assertEquals(42L, UpdateJson.integer(UpdateJson.parse("{\"size\":42}".toByteArray()), "size"))
    for (text in listOf("{\"size\":\"42\"}", "{\"size\":42.5}", "{\"size\":true}")) {
      try { UpdateJson.integer(UpdateJson.parse(text.toByteArray()), "size"); fail("Accepted non-integer") } catch (_: IllegalArgumentException) { }
    }
  }
}
