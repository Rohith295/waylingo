# JNA looks up classes and fields by name; keep them and the generated bindings.
-dontwarn java.awt.**
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
-keep class io.waylingo.** { *; }
