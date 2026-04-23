#define NOMINMAX

#include <Mod/CppUserModBase.hpp>
#include <DynamicOutput/DynamicOutput.hpp>
#include <windows.h>

#include <algorithm>
#include <chrono>
#include <cctype>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <optional>
#include <string>

using namespace RC;

class IdleCpuLimiter : public CppUserModBase {
public:
    IdleCpuLimiter() : CppUserModBase() {
        ModName = STR("IdleCpuLimiter");
        ModVersion = STR("1.0.0");
    }

    ~IdleCpuLimiter() override {}

    auto on_unreal_init() -> void override {
        m_startedAt = std::chrono::steady_clock::now();
        initializeJob();
        Output::send<LogLevel::Verbose>(STR("[IdleCpuLimiter] ready\n"));
    }

    auto on_update() -> void override {
        try {
            refreshState();
        } catch (...) {
            m_isIdle = false;
            applyCpuRate(false);
        }
    }

private:
    int m_idleCpuRate = 200;
    int m_appliedCpuRate = 0;
    bool m_isIdle = false;
    bool m_limitApplied = false;
    bool m_seenBackendLogin = false;
    bool m_seenP2pListen = false;
    bool m_bootReadyLogged = false;
    int m_idleStatusTicks = 0;
    HANDLE m_job = nullptr;
    std::uintmax_t m_logOffset = 0;
    std::optional<std::filesystem::file_time_type> m_lastStatusWriteSeen;
    std::filesystem::file_time_type m_bootReadyFileClock = std::filesystem::file_time_type::min();
    std::chrono::steady_clock::time_point m_connectionGraceUntil = std::chrono::steady_clock::time_point::min();
    std::chrono::steady_clock::time_point m_startedAt = std::chrono::steady_clock::now();
    std::chrono::steady_clock::time_point m_lastStatusRead = std::chrono::steady_clock::time_point::min();

    void initializeJob() {
        wchar_t name[128]{};
        swprintf_s(name, L"WindrosePlusIdleCpuLimiter_%lu", GetCurrentProcessId());

        m_job = CreateJobObjectW(nullptr, name);
        if (!m_job) {
            Output::send<LogLevel::Verbose>(STR("[IdleCpuLimiter] CreateJobObject failed: {}\n"), GetLastError());
            return;
        }

        if (!AssignProcessToJobObject(m_job, GetCurrentProcess())) {
            const auto err = GetLastError();
            Output::send<LogLevel::Verbose>(STR("[IdleCpuLimiter] AssignProcessToJobObject failed: {}\n"), err);
            CloseHandle(m_job);
            m_job = nullptr;
        }
    }

    void refreshState() {
        const auto now = std::chrono::steady_clock::now();
        if (m_lastStatusRead != std::chrono::steady_clock::time_point::min() &&
            now - m_lastStatusRead < std::chrono::seconds(1)) {
            return;
        }
        m_lastStatusRead = now;

        const auto dataDir = findDataDir();
        if (!dataDir) {
            m_isIdle = false;
            return;
        }

        if (std::filesystem::exists(*dataDir / "idle_cpu_limiter_disabled")) {
            m_isIdle = false;
            applyCpuRate(false);
            return;
        }

        refreshCpuRate(*dataDir);
        refreshConnectionState(*dataDir);
        refreshPlayerState(*dataDir);
        applyCpuRate(shouldLimitIdleCpu());
    }

    std::optional<std::filesystem::path> findDataDir() const {
        const std::filesystem::path candidates[] = {
            "../../../windrose_plus_data",
            "windrose_plus_data"
        };

        for (const auto& candidate : candidates) {
            std::error_code ec;
            if (std::filesystem::is_directory(candidate, ec)) {
                return candidate;
            }
        }
        return std::nullopt;
    }

    bool shouldLimitIdleCpu() const {
        const auto now = std::chrono::steady_clock::now();
        if (now < m_connectionGraceUntil) {
            return false;
        }
        if (!isBootReady() || m_idleStatusTicks < 3) {
            return false;
        }
        return m_isIdle;
    }

    void refreshConnectionState(const std::filesystem::path& dataDir) {
        const auto logPath = dataDir.parent_path() / "R5" / "Saved" / "Logs" / "R5.log";
        std::error_code ec;
        const auto size = std::filesystem::file_size(logPath, ec);
        if (ec) {
            return;
        }

        if (size < m_logOffset) {
            m_logOffset = 0;
        }

        if (size == m_logOffset) {
            return;
        }

        std::ifstream file(logPath, std::ios::binary);
        if (!file) {
            return;
        }

        file.seekg(static_cast<std::streamoff>(m_logOffset), std::ios::beg);
        std::string chunk((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
        m_logOffset = size;

        refreshBootReadiness(chunk);
        if (!containsConnectionActivity(chunk)) {
            return;
        }

        const auto now = std::chrono::steady_clock::now();
        const auto newGraceUntil = now + std::chrono::seconds(180);
        if (now >= m_connectionGraceUntil) {
            Output::send<LogLevel::Verbose>(
                STR("[IdleCpuLimiter] detected connection activity; lifting idle CPU cap for 180 seconds\n"));
        }
        m_connectionGraceUntil = newGraceUntil;
    }

    void refreshBootReadiness(const std::string& text) {
        if (text.find("Login finished successfully") != std::string::npos) {
            m_seenBackendLogin = true;
        }

        if (text.find("Initialized as an R5P2P listen server") != std::string::npos) {
            m_seenP2pListen = true;
        }

        if (isBootReady() && !m_bootReadyLogged) {
            m_bootReadyLogged = true;
            m_bootReadyFileClock = std::filesystem::file_time_type::clock::now();
            m_idleStatusTicks = 0;
            m_lastStatusWriteSeen.reset();
            Output::send<LogLevel::Verbose>(
                STR("[IdleCpuLimiter] boot readiness detected; idle CPU cap may apply after fresh idle status ticks\n"));
        }
    }

    void refreshCpuRate(const std::filesystem::path& dataDir) {
        std::ifstream file(dataDir / "idle_cpu_limiter_cpu_rate.txt");
        if (!file) {
            m_idleCpuRate = 200;
            return;
        }

        int value = 200;
        file >> value;
        m_idleCpuRate = std::clamp(value, 100, 10000);
    }

    void refreshPlayerState(const std::filesystem::path& dataDir) {
        const auto statusPath = dataDir / "server_status.json";
        std::error_code ec;
        const auto lastWrite = std::filesystem::last_write_time(statusPath, ec);
        if (ec) {
            m_isIdle = false;
            m_idleStatusTicks = 0;
            return;
        }

        const auto age = std::filesystem::file_time_type::clock::now() - lastWrite;
        if (age > std::chrono::seconds(120)) {
            m_isIdle = false;
            m_idleStatusTicks = 0;
            m_lastStatusWriteSeen.reset();
            return;
        }

        std::ifstream file(statusPath);
        if (!file) {
            m_isIdle = false;
            m_idleStatusTicks = 0;
            return;
        }

        const std::string json((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
        const auto playerCount = parsePlayerCount(json);
        m_isIdle = playerCount.has_value() && *playerCount == 0;
        if (lastWrite <= m_bootReadyFileClock) {
            m_idleStatusTicks = 0;
            return;
        }

        const bool freshStatusWrite = !m_lastStatusWriteSeen.has_value() || *m_lastStatusWriteSeen != lastWrite;
        if (freshStatusWrite) {
            m_lastStatusWriteSeen = lastWrite;
        }

        if (m_isIdle && freshStatusWrite) {
            m_idleStatusTicks = std::min(m_idleStatusTicks + 1, 100);
        } else if (!m_isIdle) {
            m_idleStatusTicks = 0;
        }
    }

    void applyCpuRate(bool shouldLimit) {
        if (!m_job || (shouldLimit == m_limitApplied && (!shouldLimit || m_idleCpuRate == m_appliedCpuRate))) {
            return;
        }

        JOBOBJECT_CPU_RATE_CONTROL_INFORMATION info{};
        info.ControlFlags = JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP;
        info.CpuRate = shouldLimit ? static_cast<DWORD>(m_idleCpuRate) : 10000;

        if (SetInformationJobObject(m_job, JobObjectCpuRateControlInformation, &info, sizeof(info))) {
            m_limitApplied = shouldLimit;
            m_appliedCpuRate = shouldLimit ? m_idleCpuRate : 10000;
            Output::send<LogLevel::Verbose>(
                STR("[IdleCpuLimiter] {} CPU rate {}\n"),
                shouldLimit ? STR("applied idle") : STR("lifted idle"),
                info.CpuRate);
        } else {
            Output::send<LogLevel::Verbose>(STR("[IdleCpuLimiter] SetInformationJobObject failed: {}\n"), GetLastError());
        }
    }

    std::optional<int> parsePlayerCount(const std::string& json) const {
        const std::string key = "\"player_count\"";
        const auto keyPos = json.find(key);
        if (keyPos == std::string::npos) {
            return std::nullopt;
        }

        const auto colonPos = json.find(':', keyPos + key.size());
        if (colonPos == std::string::npos) {
            return std::nullopt;
        }

        auto pos = colonPos + 1;
        while (pos < json.size() && std::isspace(static_cast<unsigned char>(json[pos]))) {
            ++pos;
        }

        int count = 0;
        bool foundDigit = false;
        while (pos < json.size() && std::isdigit(static_cast<unsigned char>(json[pos]))) {
            foundDigit = true;
            count = (count * 10) + (json[pos] - '0');
            ++pos;
        }

        if (!foundDigit) {
            return std::nullopt;
        }
        return count;
    }

    bool isBootReady() const {
        return m_seenBackendLogin && m_seenP2pListen;
    }

    bool containsConnectionActivity(const std::string& text) const {
        return text.find("LogNet: Login request:") != std::string::npos ||
            text.find("LogNet: Join request:") != std::string::npos ||
            text.find("OnAccountUePrelogin") != std::string::npos ||
            text.find("OnAccountUeLogin") != std::string::npos ||
            text.find("UE account verified") != std::string::npos ||
            text.find("Create R5IceConnection") != std::string::npos ||
            text.find("P2p proxy created") != std::string::npos ||
            text.find("Start connection to remote") != std::string::npos ||
            text.find("Added remote candidates") != std::string::npos ||
            text.find("Data connection is ready") != std::string::npos ||
            text.find("StartGrpcProxyServerForP2p") != std::string::npos ||
            text.find("GsStream P2pClient") != std::string::npos;
    }
};

extern "C" __declspec(dllexport) RC::CppUserModBase* start_mod() { return new IdleCpuLimiter(); }
extern "C" __declspec(dllexport) void uninstall_mod(RC::CppUserModBase* mod) { delete mod; }
