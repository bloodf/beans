package app.beans.core

import org.bouncycastle.crypto.params.Ed25519PublicKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer

internal object UpdateTrust {
  fun verify(bytes: ByteArray, signature: ByteArray, publicKey: ByteArray): Boolean {
    if (signature.size != 64 || publicKey.size != 32) return false
    return try {
      val verifier = Ed25519Signer()
      verifier.init(false, Ed25519PublicKeyParameters(publicKey, 0))
      verifier.update(bytes, 0, bytes.size)
      verifier.verifySignature(signature)
    } catch (_: IllegalArgumentException) { false }
  }

  // Multiple simultaneous signers must match exactly. Rotation must prove the installed
  // current signer in the candidate's authenticated signing history; Android checks capabilities.
  fun compatible(installed: Set<String>, candidate: Set<String>, history: Set<String>): Boolean =
    installed.isNotEmpty() && candidate.isNotEmpty() &&
      (installed == candidate || (installed.size == 1 && candidate.size == 1 && history.containsAll(installed)))
}
