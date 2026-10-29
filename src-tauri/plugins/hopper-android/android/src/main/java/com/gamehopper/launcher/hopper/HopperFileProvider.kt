package com.gamehopper.launcher.hopper

import androidx.core.content.FileProvider

/** Its own class so it never clashes with the FileProvider the app already declares. */
class HopperFileProvider : FileProvider()
