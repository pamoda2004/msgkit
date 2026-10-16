# JNA reflects over the native interface and its own classes; keep them.
-dontwarn java.awt.*
-keep class com.sun.jna.* { *; }
-keepclassmembers class * extends com.sun.jna.* { public *; }
-keep class dev.msgkit.internal.NativeLib { *; }
