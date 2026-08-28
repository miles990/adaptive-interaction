//
//  PairingStore.swift
//  InteractionCompanion
//
//  Keychain 包裝:儲存配對結果(deviceId + deviceToken + host + port + 憑證指紋)。
//  - kSecClassGenericPassword,ThisDeviceOnly(不進 iCloud 備份漫遊)。
//  - 配對碼(code)絕不儲存:僅在配對握手期間存在於記憶體。
//  - auth-fail 時「不」自動清除:只有使用者明確操作(解除配對)才清除。
//

import Foundation
import Security

/// 已配對的連線資料(不含配對碼)。
struct StoredPairing: Codable, Equatable {
    let deviceId: String
    let deviceToken: String
    let host: String
    let port: Int
    /// 伺服器自簽憑證 DER 的 SHA-256(64 位十六進位小寫)
    let fingerprint: String
}

enum PairingStoreError: LocalizedError {
    case encodeFailed
    case keychainStatus(OSStatus)

    var errorDescription: String? {
        switch self {
        case .encodeFailed:
            return "配對資料編碼失敗"
        case .keychainStatus(let status):
            return "Keychain 操作失敗(OSStatus \(status))"
        }
    }
}

final class PairingStore {
    private let service = "ai.adaptive-interaction.companion"
    private let account = "pairing.v1"

    private var baseQuery: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    /// 寫入(覆蓋既有值)。
    func save(_ pairing: StoredPairing) throws {
        let data: Data
        do {
            data = try JSONEncoder().encode(pairing)
        } catch {
            throw PairingStoreError.encodeFailed
        }

        // 先刪後加,避免 errSecDuplicateItem
        SecItemDelete(baseQuery as CFDictionary)

        var attributes = baseQuery
        attributes[kSecValueData as String] = data
        attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly

        let status = SecItemAdd(attributes as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw PairingStoreError.keychainStatus(status)
        }
    }

    /// 讀取;不存在或解碼失敗回傳 nil(解碼失敗即視為無效配對,不猜測內容)。
    func load() -> StoredPairing? {
        var query = baseQuery
        query[kSecReturnData as String] = kCFBooleanTrue as Any
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess, let data = result as? Data else {
            return nil
        }
        return try? JSONDecoder().decode(StoredPairing.self, from: data)
    }

    /// 清除。僅在使用者明確要求「解除配對」時呼叫。
    func clear() throws {
        let status = SecItemDelete(baseQuery as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw PairingStoreError.keychainStatus(status)
        }
    }
}
