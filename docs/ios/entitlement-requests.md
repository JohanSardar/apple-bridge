# Apple managed-entitlement requests

Status: prepared for Johan to paste and submit; none submitted and no approval claimed. Team `AX3C2B78YD`, iPhone bundle `com.johansardar.applebridge.ios`, app name Apple Bridge. Use the signed-in Account Holder's contact details in the forms. Apple may require an eligible organization, distribution plan, privacy policy URL, demo video or additional justification; supply actual details rather than inventing them. These requests describe the proposed gated feature, not a claim of current eligibility.

## FinanceKit financial data

Request: FinanceKit financial-data access (`com.apple.developer.financekit`). Do not request payment card provisioning or arbitrary Wallet access: FinanceKit only exposes eligible financial accounts and transactions with user consent.

Paste into the use-case / access justification field:

> Apple Bridge is a personal companion connecting an iPhone to its owner's paired gateway. We request FinanceKit financial-data access to let the owner view accounts and transactions they explicitly authorize in Apple's FinanceKit consent flow. The feature is read-only: it does not initiate payments, provision cards, or retrieve card credentials. Financial access has a separate switch that defaults off. Only the owner's authenticated, paired gateway can request data through signed, encrypted commands, with the gateway's existing authorization and approval controls. The app checks FinanceKit availability and user authorization before reading. Revoking authorization or turning off the capability prevents subsequent reads. Financial records are excluded from diagnostic evidence. The feature remains unavailable until Apple approves the entitlement and a matching provisioning profile is installed. Please advise on eligibility and any additional review requirements for this personal companion use case.

## Critical alerts

Request: Critical Alerts (`com.apple.developer.usernotifications.critical-alerts`). Approval is for eligible urgent safety use cases, not generic reminders or arbitrary high-priority gateway messages.

Paste:

> We request a review of critical-alert eligibility for Apple Bridge's proposed owner-configured urgent safety notifications. The app connects the owner's iPhone to an authenticated paired gateway. Critical alerts would be separately enabled, default off, and require both Apple's entitlement and the user's explicit critical-alert permission. They would be limited to urgent safety events selected by the owner, with ordinary notifications used for routine events. The app would not represent itself as emergency dispatch or replace emergency services. Signed, encrypted gateway commands remain subject to the existing owner authorization and approval controls. We will keep this feature unavailable unless Apple approves the use case and entitlement. Please confirm whether this use case qualifies and what supporting safety justification or demonstration you require.

## Location Push Service Extension

Request: Location Push Service Extension (`com.apple.developer.location.push`) for the main app and an extension App ID `com.johansardar.applebridge.ios.locationpush`. Register and provision the extension only after approval; a normal remote-notification entitlement is insufficient.

Paste:

> Apple Bridge requests Location Push Service Extension access for owner-initiated location refreshes from the owner's paired gateway. This is a personal device companion, not a third-party tracking service. Location sharing has a separate switch that defaults off and requires explicit system location authorization. A location push would refresh the owner's device location only while that capability remains enabled; it would not grant the sender authority to read location. Location results would travel only through the app's existing authenticated, signed and encrypted paired-gateway channel. The user can disable sharing, revoke pairing, or revoke location authorization at any time. Location data is excluded from diagnostic evidence. We will keep the extension unembedded and the feature unavailable until Apple approves and matching profiles are installed. Please advise on eligibility and any further location-sharing disclosures required.

## Fall Detection

Request: Fall Detection (`com.apple.developer.coremotion.fall-detection`). `CMFallDetectionManager` is watchOS-only; Apple approval does not make the API available on iPhone. A supported Apple Watch companion, its own registered App ID and profile, and user authorization are prerequisites. Proposed Watch bundle ID: `com.johansardar.applebridge.ios.watchkitapp`. No Watch companion is shipped by this ticket.

Paste:

> We request Fall Detection entitlement review for a proposed Apple Watch companion to Apple Bridge. With the owner's explicit authorization, a supported Apple Watch would receive Core Motion fall-detection events and relay them to the owner's paired iPhone/gateway. The feature would have a separate default-off switch, check hardware and authorization availability, and stop sharing when disabled or revoked. It would not place calls, contact emergency services, or claim to be a medical device or guaranteed emergency-monitoring service. Event information would be confined to the owner's authenticated paired devices and excluded from diagnostic evidence. We understand this is a watchOS API and requires a separately provisioned Watch app. We will keep the capability unavailable until Apple approves the entitlement and the Watch companion is implemented and qualified. Please advise whether this personal companion use case is eligible and what additional safeguards or supporting materials you require.

## After approval

Record the approval, add only the entitlement Apple granted to the appropriate target, regenerate development profiles with all required devices and certificates, and install the signed build. The managed build gates and the user's per-capability switches are separate. Never enable a build gate based only on submitting this text. FinanceKit and critical alerts additionally need on-phone consent; location push needs the extension and location authorization; fall detection needs the Watch companion. Run real qualification only after those prerequisites are met. Until then features remain `limited: needs Apple approval`.
