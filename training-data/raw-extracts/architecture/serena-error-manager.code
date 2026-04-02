import Foundation
import os.log
import Network
import Darwin

@MainActor
class ErrorManager: ObservableObject {
    @Published var currentError: SerenaError?
    @Published var isShowingError = false
    @Published var errorHistory: [ErrorRecord] = []
    @Published var isOffline = false
    
    private let logger = Logger(subsystem: "com.serenanet.app", category: "ErrorManager")
    private let networkLogger = Logger(subsystem: "com.serenanet.app", category: "Network")
    private let loggingManager = LoggingManager.shared
    private let maxErrorHistory = 50
    private let networkMonitor = NWPathMonitor()
    private let networkQueue = DispatchQueue(label: "NetworkMonitor")
    
    struct ErrorRecord: Identifiable {
        let id = UUID()
        let error: SerenaError
        let timestamp: Date
        let context: String?
        let severity: ErrorSeverity
        let wasRecovered: Bool
        
        init(error: SerenaError, context: String? = nil, wasRecovered: Bool = false) {
            self.error = error
            self.timestamp = Date()
            self.context = context
            self.severity = error.severity
            self.wasRecovered = wasRecovered
        }
    }
    
    // MARK: - Initialization
    
    init() {
        startNetworkMonitoring()
    }
    
    deinit {
        networkMonitor.cancel()
    }
    
    // MARK: - Error Handling
    
    func handle(_ error: SerenaError, context: String? = nil) {
        logError(error, context: context)
        
        let record = ErrorRecord(error: error, context: context)
        errorHistory.insert(record, at: 0)
        
        // Limit error history size
        if errorHistory.count > maxErrorHistory {
            errorHistory = Array(errorHistory.prefix(maxErrorHistory))
        }
        
        // Show error to user based on severity
        if error.severity != .info {
            currentError = error
            isShowingError = true
        }
    }
    
    func handle(_ error: Error, context: String? = nil) {
        let serenaError: SerenaError
        
        if let serenaErr = error as? SerenaError {
            serenaError = serenaErr
        } else {
            serenaError = .unknownError(error.localizedDescription)
        }
        
        handle(serenaError, context: context)
    }
    
    func dismissError() {
        currentError = nil
        isShowingError = false
    }
    
    // MARK: - Error Recovery
    
    func getRecoveryAction(for error: SerenaError) -> ErrorRecoveryAction {
        switch error {
        case .aiModelNotLoaded:
            return .wait("Please wait for the AI model to load")
        case .aiModelNotFound:
            return .userIntervention("Install the required AI model files")
        case .aiModelInitializationFailed:
            return .restart("Restart the application")
        case .aiResponseGenerationFailed:
            return .retry("Try sending your message again")
        case .aiProcessingError:
            return .retry("Try sending your message again")
        case .voicePermissionDenied:
            return .userIntervention("Enable microphone permission in System Preferences")
        case .voiceRecognitionFailed:
            return .retry("Try speaking again")
        case .voiceRecognitionUnavailable:
            return .userIntervention("Voice recognition is not available on this device")
        case .voiceRecognitionSetupFailed:
            return .retry("Try restarting voice input")
        case .databaseError:
            return .restart("Restart the application")
        case .networkUnavailable:
            return .ignore("Continue using offline mode")
        case .invalidFTAIFormat:
            return .retry("Check the file format and try again")
        case .configurationError:
            return .userIntervention("Reset settings to defaults")
        case .conversationNotFound:
            return .retry("Select a different conversation or create a new one")
        case .emptyMessage:
            return .ignore("Please enter a message before sending")
        case .unknownError:
            return .restart("Restart the application")
        }
    }
    
    // MARK: - Network Monitoring
    
    private func startNetworkMonitoring() {
        networkMonitor.pathUpdateHandler = { [weak self] path in
            DispatchQueue.main.async {
                self?.updateNetworkStatus(path.status == .satisfied)
            }
        }
        networkMonitor.start(queue: networkQueue)
    }
    
    private func updateNetworkStatus(_ isConnected: Bool) {
        let wasOffline = isOffline
        isOffline = !isConnected
        
        if wasOffline && isConnected {
            networkLogger.info("Network connectivity restored")
        } else if !wasOffline && !isConnected {
            networkLogger.notice("Network connectivity lost - entering offline mode")
            handle(.networkUnavailable, context: "Network connectivity lost")
        }
    }
    
    // MARK: - Logging
    
    private func logError(_ error: SerenaError, context: String?) {
        let contextString = context ?? "Unknown context"
        let errorId = UUID().uuidString.prefix(8)
        let message = "[\(errorId)] \(error.localizedDescription) | Context: \(contextString) | Recoverable: \(error.isRecoverable)"
        
        // Log to both os.log and LoggingManager
        switch error.severity {
        case .info:
            logger.info("\(message)")
            loggingManager.log(message, category: .error, level: .info)
        case .warning:
            logger.notice("\(message)")
            loggingManager.log(message, category: .error, level: .warning)
        case .error:
            logger.error("\(message)")
            loggingManager.log(message, category: .error, level: .error)
        case .critical:
            logger.critical("\(message)")
            loggingManager.log(message, category: .error, level: .fault)
        }
        
        // Additional diagnostic information for critical errors
        if error.severity == .critical {
            let diagnosticMessage = "System state - Memory pressure: \(self.getMemoryPressure()), Offline: \(self.isOffline)"
            logger.critical("\(diagnosticMessage)")
            loggingManager.log(diagnosticMessage, category: .error, level: .fault)
        }
    }
    
    private func getMemoryPressure() -> String {
        var info = mach_task_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size)/4
        
        let kerr: kern_return_t = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: 1) {
                task_info(mach_task_self_,
                         task_flavor_t(MACH_TASK_BASIC_INFO),
                         $0,
                         &count)
            }
        }
        
        if kerr == KERN_SUCCESS {
            let memoryMB = info.resident_size / (1024 * 1024)
            return "\(memoryMB)MB"
        }
        return "Unknown"
    }
    
    // MARK: - Error Statistics
    
    func getErrorCount(for severity: ErrorSeverity) -> Int {
        return errorHistory.filter { $0.error.severity == severity }.count
    }
    
    func getRecentErrors(limit: Int = 10) -> [ErrorRecord] {
        return Array(errorHistory.prefix(limit))
    }
    
    func clearErrorHistory() {
        errorHistory.removeAll()
    }
    
    // MARK: - Recovery Tracking
    
    func markErrorAsRecovered(_ errorId: UUID) {
        if let index = errorHistory.firstIndex(where: { $0.id == errorId }) {
            let originalRecord = errorHistory[index]
            let recoveredRecord = ErrorRecord(
                error: originalRecord.error,
                context: originalRecord.context,
                wasRecovered: true
            )
            errorHistory[index] = recoveredRecord
            logger.info("Error \(errorId.uuidString.prefix(8)) marked as recovered")
        }
    }
    
    // MARK: - Diagnostic Information
    
    func generateDiagnosticReport() -> String {
        let formatter = DateFormatter()
        formatter.dateStyle = .short
        formatter.timeStyle = .medium
        
        var report = "=== SerenaNet Error Diagnostic Report ===\n"
        report += "Generated: \(formatter.string(from: Date()))\n"
        report += "Network Status: \(isOffline ? "Offline" : "Online")\n"
        report += "Memory Usage: \(getMemoryPressure())\n"
        report += "Total Errors: \(errorHistory.count)\n\n"
        
        let errorCounts = ErrorSeverity.allCases.map { severity in
            let count = getErrorCount(for: severity)
            return "\(severity): \(count)"
        }.joined(separator: ", ")
        report += "Error Breakdown: \(errorCounts)\n\n"
        
        report += "Recent Errors:\n"
        for (index, record) in getRecentErrors(limit: 10).enumerated() {
            let status = record.wasRecovered ? "✓ Recovered" : "⚠ Unresolved"
            report += "\(index + 1). [\(formatter.string(from: record.timestamp))] \(record.error.localizedDescription) - \(status)\n"
            if let context = record.context {
                report += "   Context: \(context)\n"
            }
        }
        
        return report
    }
    
    // MARK: - User Guidance
    
    func getUserGuidanceMessage(for error: SerenaError) -> String {
        let baseMessage = error.recoverySuggestion ?? "Please try again."
        
        switch error {
        case .aiModelNotLoaded:
            return "\(baseMessage)\n\nTip: The AI model typically takes 10-30 seconds to load on first launch."
        case .voicePermissionDenied:
            return "\(baseMessage)\n\nNote: You can also type your messages if voice input isn't available."
        case .networkUnavailable:
            return "\(baseMessage)\n\nGood news: All AI processing happens locally, so you can continue chatting normally."
        case .databaseError:
            return "\(baseMessage)\n\nWarning: This may result in loss of conversation history. Consider backing up your data."
        default:
            return baseMessage
        }
    }
}

enum ErrorRecoveryAction {
    case retry(String)
    case wait(String)
    case restart(String)
    case userIntervention(String)
    case ignore(String)
    
    var actionText: String {
        switch self {
        case .retry(let text), .wait(let text), .restart(let text), .userIntervention(let text), .ignore(let text):
            return text
        }
    }
    
    var buttonText: String {
        switch self {
        case .retry:
            return "Retry"
        case .wait:
            return "OK"
        case .restart:
            return "Restart"
        case .userIntervention:
            return "Open Settings"
        case .ignore:
            return "Continue"
        }
    }
}