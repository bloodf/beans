import type { ExpoConfig } from "expo/config";
import releasePackage from "../package.json";

export default (): ExpoConfig => {
  const development =
    process.env.LORCA_MOBILE_VARIANT === "development" ||
    process.env.EAS_BUILD_PROFILE === "development";
  const appName = development ? "Beans Dev" : "Beans";
  const appId = development ? "ai.amoena.beans.dev" : "ai.amoena.beans";
  const appGroup = development ? "group.ai.amoena.beans.dev" : "group.ai.amoena.beans";
  const icon = development ? "./assets/icon-dev.png" : "./assets/icon.png";
  const splash = development ? "./assets/splash-icon-dev.png" : "./assets/splash-icon.png";
  const favicon = development ? "./assets/favicon-dev.png" : "./assets/favicon.png";
  const adaptiveIconBackgroundColor = development ? "#ffbe00" : "#3424f5";
  const buildNumber = process.env.BUILD_NUMBER ?? String(Math.floor(Date.now() / 1000));
  if (!/^[1-9]\d*$/.test(buildNumber) || Number(buildNumber) > 2_100_000_000) {
    throw new Error("BUILD_NUMBER must be a positive integer at most 2100000000");
  }
  const splashBackgroundColor = "#f7f7f8";
  const darkSplashBackgroundColor = "#1c1c1e";

  return {
    name: appName,
    slug: "beans",
    version: releasePackage.version,
    scheme: development ? ["beans-dev", "lorca"] : ["beans", "lorca"],
    orientation: "portrait",
    icon,
    userInterfaceStyle: "automatic",
    ios: {
      bundleIdentifier: appId,
      supportsTablet: true,
      infoPlist: {
        NSCameraUsageDescription: "Beans scans a pairing QR code from another Device.",
        NSMicrophoneUsageDescription: "Beans listens while you dictate a message.",
        NSSpeechRecognitionUsageDescription: "Beans turns what you say into the message text.",
        NSPhotoLibraryUsageDescription: "Beans attaches photos you pick to a message.",
        CFBundleAllowMixedLocalizations: true,
      },
      // GitHub builds share a monotonic BUILD_NUMBER with Android; local builds use epoch seconds.
      buildNumber: process.env.LORCA_IOS_BUILD_NUMBER ?? buildNumber,
      entitlements: {
        "com.apple.security.application-groups": [appGroup],
      },
      appleTeamId: process.env.BEANS_APPLE_TEAM_ID ?? "GJE9R5VE87",
    },
    android: {
      package: appId,
      // FCM tokens for pushes: set BEANS_GOOGLE_SERVICES_FILE to your Firebase google-services.json.
      ...(process.env.BEANS_GOOGLE_SERVICES_FILE ? { googleServicesFile: process.env.BEANS_GOOGLE_SERVICES_FILE } : {}),
      adaptiveIcon: {
        backgroundColor: adaptiveIconBackgroundColor,
        foregroundImage: icon,
      },
      predictiveBackGestureEnabled: true,
      versionCode: Number(buildNumber),
    },
    web: {
      favicon,
    },
    plugins: [
      "expo-router",
      "expo-secure-store",
      "expo-system-ui",
      [
        "expo-splash-screen",
        {
          backgroundColor: splashBackgroundColor,
          image: splash,
          imageWidth: 220,
          resizeMode: "contain",
          dark: {
            backgroundColor: darkSplashBackgroundColor,
            image: splash,
          },
        },
      ],
      [
        "expo-camera",
        {
          cameraPermission: "Beans scans a pairing QR code from another Device.",
        },
      ],
      [
        "expo-image-picker",
        {
          photosPermission: "Beans attaches photos you pick to a message.",
          cameraPermission: "Beans attaches photos you take to a message.",
        },
      ],
      [
        "expo-speech-recognition",
        {
          microphonePermission: "Beans listens while you dictate a message.",
          speechRecognitionPermission: "Beans turns what you say into the message text.",
        },
      ],
      "expo-image",
      "expo-localization",
      "expo-notifications",
      "@bacons/apple-targets",
      "./plugins/with-scene-lifecycle",
      "./plugins/with-android-release-signing",
      "./plugins/with-android-locale-defaults",
      "./plugins/with-ios-release-signing",
      "expo-web-browser",
    ],
    experiments: {
      typedRoutes: true,
    },
    locales: {
      en: "./locales/en.json",
      "zh-Hans": "./locales/zh-Hans.json",
    },
  };
};
