package app.beans.core

import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener

internal object UpdateJson {
  // Reject duplicate JSON object keys, including nested inventory/provenance objects.
  fun parse(bytes: ByteArray): JSONObject {
    val text = bytes.toString(Charsets.UTF_8)
    val token = JSONTokener(text)
    fun parse(t: JSONTokener): Any? {
      return when (val c = t.nextClean()) {
        '{' -> {
          val obj = JSONObject(); val keys = mutableSetOf<String>()
          if (t.nextClean() == '}') obj else {
            t.back()
            while (true) {
              require(t.nextClean() == '"'); t.back()
              val key = t.nextValue() as String; require(keys.add(key)) { "Duplicate JSON key" }
              require(t.nextClean() == ':'); obj.put(key, parse(t))
              val end = t.nextClean(); if (end == '}') break
              require(end == ',')
            }; obj
          }
        }
        '[' -> {
          val arr = JSONArray()
          if (t.nextClean() == ']') arr else {
            t.back()
            while (true) { arr.put(parse(t)); val end = t.nextClean(); if (end == ']') break; require(end == ',') }; arr
          }
        }
        else -> { t.back(); t.nextValue() }
      }
    }
    val result = parse(token) as JSONObject
    require(token.nextClean() == '\u0000') { "Trailing JSON bytes" }
    return result
  }
  fun integer(obj: JSONObject, key: String): Long {
    val value = obj.get(key)
    require(value is Int || value is Long) { "Invalid integer: $key" }
    return (value as Number).toLong()
  }
}
