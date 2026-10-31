# iPadOS distribution status

This community modification is a source-build preview awaiting upstream review, not a
publicly installable iPadOS release. Do not upload a Personal Team development build as a
general-purpose installer. Its provisioning is not valid for arbitrary users' devices and
the bundle/profile can disclose signing and registered-device information.

## Installation without Xcode for testers

The intended beta route is **TestFlight**, managed by a willing publisher/maintainer with
Apple Developer Program membership and App Store Connect access. Testers install Apple's
TestFlight app and follow an invitation; they do not need Xcode or a Mac. External beta
distribution is subject to Apple's review. No such account, submission or invitation is
created by this patch. See [Apple's TestFlight overview](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/)
and [distribution requirements](https://developer.apple.com/documentation/xcode/distributing-your-app-for-beta-testing-and-releases).

Direct website installation is not equivalent to hosting an IPA on GitHub. Apple's
[EU Web Distribution](https://developer.apple.com/support/web-distribution-eu/) requires
developer eligibility, notarization and a registered website domain, among other conditions.
Ad Hoc distribution is limited to registered devices; it is not a public installer.

## Work required before a public beta

1. Obtain upstream review and agree who publishes and maintains the iPad port.
2. Fix/document the missing brush outline and complete document import/save/share. The current
   shell does not provide the iPadOS Files picker, so it is not suitable for valuable documents.
3. Test supported devices/OS versions, background/resume, rotation, Pencil contact/hover and
   interruption recovery. The tested device/OS combination is listed in the README.
4. Supply a properly licensed iPad app icon, review privacy manifests/API requirements and
   third-party licenses, and prepare App Store Connect metadata and beta review information.
5. Configure the publisher's own bundle ID and signing locally or in a protected CI environment.
   Keep certificates, private keys, profiles, API credentials and device lists out of Git.
6. Produce and validate a Release archive, upload through Xcode Organizer/App Store Connect,
   then obtain the required Apple review before inviting external testers.

The scheme includes a Release archive action. A publisher can begin with the following command
after setting up their own ignored `Signing.local.xcconfig`:

```sh
xcodebuild -project packaging/ios/PhotoCraft.xcodeproj -scheme PhotoCraft \
  -configuration Release -destination 'generic/platform=iOS' \
  -archivePath target/PhotoCraft-iPad.xcarchive \
  -allowProvisioningUpdates archive
```

This documents the release path; an App Store/TestFlight archive has **not** been validated or
submitted. A successful Debug build is not evidence of App Store readiness. Signing, upload,
Apple agreements and publication remain the publisher's responsibility.
