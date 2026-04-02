import Foundation

public enum SerenaError: LocalizedError, Equatable {
    case aiModelNotLoaded
    case aiModelNotFound(String)
    case aiModelInitializationFailed(String)
    case aiResponseGenerationFailed(String)
    case aiProcessingError(String)
    case voicePermissionDenied
    case voiceRecognitionFailed(String)
    case voiceRecognitionUnavailable
    case voiceRecognitionSetupFailed
    case databaseError(String)
    case networkUnavailable
    case invalidFTAIFormat(String)
    case configurationError(String)
    case conversationNotFound
    case emptyMessage
    case unknownError(String)
    
    public var errorDescription: String? {
        switch self {
        case .aiModelNotLoaded:
            return "AI model is not ready. Please wait for initialization to complete."
        case .aiModelNotFound(let message):
            return "AI model files not found: \(message)"
        case .aiModelInitializationFailed(let message):
            return "Failed to initialize AI model: \(message)"
        case .aiResponseGenerationFailed(let message):
            return "Failed to generate AI response: \(message)"
        case .aiProcessingError(let message):
            return "AI processing error: \(message)"
        case .voicePermissionDenied:
            return "Voice input requires microphone permission. Please enable it in System Preferences."
        case .voiceRecognitionFailed(let message):
            return "Voice recognition failed: \(message)"
        case .voiceRecognitionUnavailable:
            return "Voice recognition is not available on this device or language."
        case .voiceRecognitionSetupFailed:
            return "Failed to set up voice recognition. Please try again."
        case .databaseError(let message):
            return "Database error: \(message)"
        case .networkUnavailable:
            return "Network is unavailable, but SerenaNet continues working offline."
        case .invalidFTAIFormat(let message):
            return "Invalid FTAI file format: \(message)"
        case .configurationError(let message):
            return "Configuration error: \(message)"
        case .conversationNotFound:
            return "The requested conversation could not be found."
        case .emptyMessage:
            return "Message cannot be empty."
        case .unknownError(let message):
            return "An unexpected error occurred: \(message)"
        }
    }
    
    public var recoverySuggestion: String? {
        switch self {
        case .aiModelNotLoaded:
            return "Please wait a moment for the AI model to finish loading."
        case .aiModelNotFound:
            return "Please ensure the Mixtral model files are properly installed. Check the documentation for installation instructions."
        case .aiModelInitializationFailed:
            return "Try restarting the application. If the problem persists, check that your device meets the system requirements."
        case .aiResponseGenerationFailed:
            return "Try rephrasing your message or starting a new conversation."
        case .aiProcessingError:
            return "Try sending your message again or starting a new conversation."
        case .voicePermissionDenied:
            return "Go to System Preferences > Security & Privacy > Privacy > Microphone and enable access for SerenaNet."
        case .voiceRecognitionFailed:
            return "Try speaking more clearly or check your microphone connection."
        case .voiceRecognitionUnavailable:
            return "Try using a different language or check if your device supports speech recognition."
        case .voiceRecognitionSetupFailed:
            return "Try restarting the application or check your microphone connection."
        case .databaseError:
            return "Try restarting the application. If the problem persists, you may need to reset your data."
        case .networkUnavailable:
            return "SerenaNet works offline, so you can continue using it normally."
        case .invalidFTAIFormat:
            return "Check the FTAI file format and try again."
        case .configurationError:
            return "Try resetting your settings to defaults in the Settings menu."
        case .conversationNotFound:
            return "Try selecting a different conversation or creating a new one."
        case .emptyMessage:
            return "Please enter a message before sending."
        case .unknownError:
            return "Try restarting the application. If the problem persists, please report this issue."
        }
    }
    
    var isRecoverable: Bool {
        switch self {
        case .aiModelNotLoaded, .voicePermissionDenied, .voiceRecognitionUnavailable, .voiceRecognitionSetupFailed, .networkUnavailable, .conversationNotFound, .emptyMessage:
            return true
        case .aiModelNotFound, .aiModelInitializationFailed, .databaseError, .configurationError:
            return false
        case .aiResponseGenerationFailed, .aiProcessingError, .voiceRecognitionFailed, .invalidFTAIFormat, .unknownError:
            return true
        }
    }
    
    var severity: ErrorSeverity {
        switch self {
        case .networkUnavailable, .emptyMessage:
            return .info
        case .voicePermissionDenied, .voiceRecognitionFailed, .voiceRecognitionUnavailable, .voiceRecognitionSetupFailed, .invalidFTAIFormat, .conversationNotFound:
            return .warning
        case .aiModelNotLoaded, .aiResponseGenerationFailed, .aiProcessingError:
            return .error
        case .aiModelNotFound, .aiModelInitializationFailed, .databaseError, .configurationError, .unknownError:
            return .critical
        }
    }
}

enum ErrorSeverity: CaseIterable {
    case info
    case warning
    case error
    case critical
    
    var systemImageName: String {
        switch self {
        case .info:
            return "info.circle"
        case .warning:
            return "exclamationmark.triangle"
        case .error:
            return "xmark.circle"
        case .critical:
            return "exclamationmark.octagon"
        }
    }
    
    var color: String {
        switch self {
        case .info:
            return "blue"
        case .warning:
            return "orange"
        case .error:
            return "red"
        case .critical:
            return "red"
        }
    }
}