package app.beans.core

import org.junit.Assert.*
import org.junit.Test

class UpdateTrustTest {
  // RFC 8032 test vector 1: public key and detached signature of empty bytes.
  private fun hex(value: String) = value.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
  private val key = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
  private val signature = hex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")

  @Test fun authenticBytesOnly() {
    assertTrue(UpdateTrust.verify(byteArrayOf(), signature, key))
    assertFalse(UpdateTrust.verify(byteArrayOf(1), signature, key))
    assertFalse(UpdateTrust.verify(byteArrayOf(), signature.copyOf().also { it[0] = 0 }, key))
    assertFalse(UpdateTrust.verify(byteArrayOf(), signature, key.copyOf().also { it[0] = 0 }))
    assertFalse(UpdateTrust.verify(byteArrayOf(), signature.copyOf(63), key))
  }

  @Test fun signerCompatibilityUsesInstalledIdentity() {
    assertTrue(UpdateTrust.compatible(setOf("a"), setOf("a"), setOf("a")))
    assertTrue(UpdateTrust.compatible(setOf("a"), setOf("b"), setOf("a", "b")))
    assertFalse(UpdateTrust.compatible(setOf("a"), setOf("b"), setOf("b")))
    assertFalse(UpdateTrust.compatible(setOf("a", "b"), setOf("a"), setOf("a", "b")))
    assertFalse(UpdateTrust.compatible(emptySet(), setOf("a"), setOf("a")))
  }
}
