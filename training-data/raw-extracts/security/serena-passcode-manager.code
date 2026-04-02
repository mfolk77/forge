import Foundation
import CryptoKit
import Security
import LocalAuthentication

@MainActor
class PasscodeManager: ObservableObject {
    @Published var isLocked: Bool = false
    @Published var passcodeEnabled: Bool = false
    
    private let keychainService = "SerenaNet-Passcode"
    private let keychainAccount = "user-passcode"
    
    init() {
        checkPasscodeStatus()
    }
    
    // MARK: - Passcode Status
    
    func checkPasscodeStatus() {
        passcodeEnabled = hasStoredPasscode()
        isLocked = passcodeEnabled
    }
    
    private func hasStoredPasscode() -> Bool {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: keychainAccount,
            kSecReturnData as String: false
        ]
        
        let status = SecItemCopyMatching(query as CFDictionary, nil)
        return status == errSecSuccess
    }
    
    // MARK: - Passcode Management
    
    func setPasscode(_ passcode: String) throws {
        guard !passcode.isEmpty else {
            throw PasscodeError.emptyPasscode
        }
        
        guard passcode.count >= 4 else {
            throw PasscodeError.passcodeTooShort
        }
        
        let hashedPasscode = hashPasscode(passcode)
        try storePasscodeHash(hashedPasscode)
        
        passcodeEnabled = true
        isLocked = false
    }
    
    func removePasscode() throws {
        try deleteStoredPasscode()
        passcodeEnabled = false
        isLocked = false
    }
    
    func verifyPasscode(_ passcode: String) throws -> Bool {
        guard let storedHash = try getStoredPasscodeHash() else {
            throw PasscodeError.noPasscodeSet
        }
        
        let inputHash = hashPasscode(passcode)
        let isValid = storedHash == inputHash
        
        if isValid {
            isLocked = false
        }
        
        return isValid
    }
    
    func lockApp() {
        if passcodeEnabled {
            isLocked = true
        }
    }
    
    // MARK: - Biometric Authentication
    
    func authenticateWithBiometrics() async throws -> Bool {
        let context = LAContext()
        var error: NSError?
        
        guard context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error) else {
            throw PasscodeError.biometricsNotAvailable
        }
        
        do {
            let success = try await context.evaluatePolicy(
                .deviceOwnerAuthenticationWithBiometrics,
                localizedReason: "Authenticate to access SerenaNet"
            )
            
            if success {
                isLocked = false
            }
            
            return success
        } catch {
            throw PasscodeError.biometricAuthenticationFailed(error)
        }
    }
    
    func canUseBiometrics() -> Bool {
        let context = LAContext()
        return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
    }
    
    // MARK: - Private Methods
    
    private func hashPasscode(_ passcode: String) -> String {
        let data = Data(passcode.utf8)
        let hash = SHA256.hash(data: data)
        return hash.compactMap { String(format: "%02x", $0) }.joined()
    }
    
    private func storePasscodeHash(_ hash: String) throws {
        let data = Data(hash.utf8)
        
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: keychainAccount,
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        ]
        
        // Delete existing item first
        SecItemDelete(query as CFDictionary)
        
        let status = SecItemAdd(query as CFDictionary, nil)
        
        if status != errSecSuccess {
            throw PasscodeError.keychainError(status)
        }
    }
    
    private func getStoredPasscodeHash() throws -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: keychainAccount,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        
        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        
        if status == errSecSuccess {
            guard let data = result as? Data,
                  let hash = String(data: data, encoding: .utf8) else {
                throw PasscodeError.invalidKeychainData
            }
            return hash
        } else if status == errSecItemNotFound {
            return nil
        } else {
            throw PasscodeError.keychainError(status)
        }
    }
    
    private func deleteStoredPasscode() throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecAttrAccount as String: keychainAccount
        ]
        
        let status = SecItemDelete(query as CFDictionary)
        
        if status != errSecSuccess && status != errSecItemNotFound {
            throw PasscodeError.keychainError(status)
        }
    }
    
    // MARK: - Memory Protection
    
    func clearSensitiveMemory() {
        // Force garbage collection and clear any cached sensitive data
        // This is a best-effort approach as Swift doesn't provide direct memory control
        autoreleasepool {
            // Any temporary sensitive data would be cleared here
        }
    }
}

// MARK: - Error Types

enum PasscodeError: LocalizedError {
    case emptyPasscode
    case passcodeTooShort
    case noPasscodeSet
    case invalidPasscode
    case keychainError(OSStatus)
    case invalidKeychainData
    case biometricsNotAvailable
    case biometricAuthenticationFailed(Error)
    
    var errorDescription: String? {
        switch self {
        case .emptyPasscode:
            return "Passcode cannot be empty"
        case .passcodeTooShort:
            return "Passcode must be at least 4 characters long"
        case .noPasscodeSet:
            return "No passcode has been set"
        case .invalidPasscode:
            return "Invalid passcode"
        case .keychainError(let status):
            return "Keychain error: \(status)"
        case .invalidKeychainData:
            return "Invalid data retrieved from keychain"
        case .biometricsNotAvailable:
            return "Biometric authentication is not available on this device"
        case .biometricAuthenticationFailed(let error):
            return "Biometric authentication failed: \(error.localizedDescription)"
        }
    }
}