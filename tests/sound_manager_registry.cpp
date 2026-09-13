#include "sound_manager.h"
#include "bflib_sndlib.h"
#include "bflib_fileio.h"
#include "config_mods.h"
#include "custom_zip.h"
#include "game_legacy.h"
#include <cstdlib>
#include <cstdio>
#include <cstring>
#include <string>
#include <unordered_map>
#include <vector>

static std::vector<std::string> bank;
static std::unordered_map<std::string, std::string> files;
static std::unordered_map<std::string, std::string> zip_entries;
static std::string path;
static ModsConfig test_mods{};
struct Game game{};

extern "C" {
int custom_sound_bank_size() { return static_cast<int>(bank.size()); }
SoundSmplTblID get_custom_offset() { return 1000; }
void custom_sound_bank_clear() { bank.clear(); }
TbBool custom_sound_load_wav(const char* filepath, int sample_id) {
    auto found = files.find(filepath);
    if (found == files.end() || found->second == "invalid") return false;
    if (sample_id != static_cast<int>(bank.size())) std::abort();
    bank.push_back(found->second);
    return true;
}
TbBool custom_sound_load_wav_mem(const unsigned char* data, size_t size, const char*, int sample_id) {
    if (size == 0 || data[0] == '!') return false;
    if (sample_id != static_cast<int>(bank.size())) std::abort();
    bank.emplace_back(reinterpret_cast<const char*>(data), size);
    return true;
}
TbBool GetSoundInstalled() { return true; }
TbBool init_sound() { return true; }
int LbWarnLog(const char*, ...) { return 0; }
GameTurn get_gameturn() { return 0; }
short LbFileExists(const char* filename) { return files.count(filename) != 0; }
char* prepare_file_path(short group, const char* filename) {
    path = std::to_string(group) + "/" + filename;
    return path.data();
}
char* prepare_file_path_mod(const char* mod_dir, short, const char* filename) {
    path = std::string(mod_dir) + "/sound/" + filename;
    return path.data();
}
const ModsConfig* get_loaded_mods_conf() { return &test_mods; }
TbBool read_map_zip_entry(LevelNumber, const char* name, unsigned char** out, size_t* size) {
    auto found = zip_entries.find(name);
    if (found == zip_entries.end()) return false;
    *size = found->second.size();
    *out = static_cast<unsigned char*>(malloc(*size));
    memcpy(*out, found->second.data(), *size);
    return true;
}
}

static void check(bool condition, const char* message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message);
        std::exit(1);
    }
}

static void expect(const char* name, SoundSmplTblID id, int count) {
    check(sound_manager_get_id(name) == id, name);
    check(sound_manager_get_count(name) == count, "variant count matches active mapping");
}

static void reset() {
    sound_manager_clear_custom_sounds();
    sound_manager_clear_registry();
    files.clear();
    zip_entries.clear();
    test_mods = {};
}

static void disk(const char* name, const char* content) {
    files[std::to_string(FGrp_FxData) + "/" + name] = content;
}

int main() {
    auto& sm = KeeperFX::SoundManager::getInstance();
    reset();
    sound_manager_register("TAB_CLICK", 9, 2);
    files["after-base.wav"] = "after-base";
    const auto base = sm.loadCustomSound("TAB_CLICK", "after-base.wav");
    check(sound_manager_get_id("TAB_CLICK") == base, "custom replaces numeric mapping");
    check(base == 1000, "first custom bank ID");
    sound_manager_register("TAB_CLICK", 15, 3);
    expect("TAB_CLICK", 15, 3);
    const auto reused = sm.loadCustomSound("TAB_CLICK", "after-base.wav");
    check(reused == base && bank.size() == 1, "same file reuses buffer");
    expect("TAB_CLICK", base, 1);
    files["campaign.wav"] = "campaign";
    const auto campaign = sm.loadCustomSound("TAB_CLICK", "campaign.wav");
    check(campaign != base && bank[campaign - 1000] == "campaign", "different file overrides custom");
    expect("TAB_CLICK", campaign, 1);
    check(sm.loadCustomSound("TAB_CLICK", "missing.wav") == -1, "missing override fails");
    expect("TAB_CLICK", campaign, 1);

    sound_manager_save_snapshot();
    const auto watermark = bank.size();
    files["map.wav"] = "map";
    const auto map = sm.loadCustomSound("TAB_CLICK", "map.wav");
    expect("TAB_CLICK", map, 1);
    sound_manager_register("TAB_CLICK", 23, 4);
    expect("TAB_CLICK", 23, 4);
    sound_manager_restore_snapshot();
    bank.resize(watermark);
    expect("TAB_CLICK", campaign, 1);
    check(sm.getCustomSoundId("TAB_CLICK") == campaign, "snapshot restores custom source");
    check(sm.loadCustomSound("TAB_CLICK", "campaign.wav") == campaign, "restored source cache valid");
    sound_manager_register("TAB_CLICK", 12, 2);
    sound_manager_save_snapshot();
    sm.loadCustomSound("TAB_CLICK", "campaign.wav");
    sound_manager_restore_snapshot();
    expect("TAB_CLICK", 12, 2);
    sound_manager_register("TAB_CLICK", campaign, 1);
    sound_manager_clear_custom_sounds();
    sound_manager_restore_snapshot();
    check(sound_manager_get_id("TAB_CLICK") == 0, "clearing bank invalidates snapshot and custom registry IDs");

    reset();
    disk("click.wav", "file");
    const auto file = sound_manager_load_named_sound("TAB_CLICK", "click.wav", 1);
    expect("TAB_CLICK", file, 1);
    zip_entries["sound/map.wav"] = "zip-one";
    const auto zip = sound_manager_load_named_sound("TAB_CLICK", "map.wav", 1);
    expect("TAB_CLICK", zip, 1);
    check(bank[zip - 1000] == "zip-one", "zip overrides file");
    check(sound_manager_load_named_sound("TAB_CLICK", "map.wav", 1) == zip, "identical zip bytes reuse");
    zip_entries["sound/map.wav"] = "zip-two";
    const auto zip2 = sound_manager_load_named_sound("TAB_CLICK", "map.wav", 1);
    check(zip2 != zip && bank[zip2 - 1000] == "zip-two", "changed same-size zip bytes override");
    zip_entries["sound/map.wav"] = "!invalid";
    check(sound_manager_load_named_sound("TAB_CLICK", "map.wav", 1) == 0, "invalid zip fails");
    expect("TAB_CLICK", zip2, 1);
    check(sound_manager_load_named_sound("TAB_CLICK", "click.wav", 1) > zip2, "file overrides zip");
    sound_manager_register("TAB_CLICK", 7, 1);
    sound_manager_clear_custom_sounds();
    expect("TAB_CLICK", 7, 1);

    reset();
    disk("hit01.wav", "one");
    disk("hit02.wav", "two");
    disk("hit03.wav", "three");
    disk("single.wav", "single");
    const auto single = sound_manager_load_named_sound("HIT", "single.wav", 1);
    const auto family = sound_manager_load_named_sound("HIT", "hit01.wav", 3);
    check(family > single, "family replaces single custom");
    expect("HIT", family, 3);
    check(bank[family - 1000] == "one" && bank[family - 999] == "two" && bank[family - 998] == "three", "family contiguous");
    const auto family_size = bank.size();
    check(sound_manager_load_named_sound("HIT", "hit01.wav", 3) == family && bank.size() == family_size,
          "identical contiguous family reuses buffers");
    files["replacement.wav"] = "replacement";
    sm.loadCustomSound("HIT_1", "replacement.wav");
    const auto reloaded = sound_manager_load_named_sound("HIT", "hit01.wav", 3);
    expect("HIT", reloaded, 3);
    check(bank[reloaded - 1000] == "one" && bank[reloaded - 999] == "two" && bank[reloaded - 998] == "three", "cached variants cannot scatter family");
    sound_manager_save_snapshot();
    const auto family_watermark = bank.size();
    const auto variant0 = sm.getCustomSoundId("HIT_0");
    disk("map01.wav", "map-one");
    disk("map02.wav", "invalid");
    disk("map03.wav", "map-three");
    check(sound_manager_load_named_sound("HIT", "map01.wav", 3) == 0, "incomplete family rejected");
    expect("HIT", reloaded, 3);
    check(sm.getCustomSoundId("HIT_0") == variant0, "failed family rolls back variant registry and cache");
    expect("HIT_0", variant0, 1);
    sound_manager_register("HIT", 8, 2);
    expect("HIT", 8, 2);
    const auto new_single = sound_manager_load_named_sound("HIT", "single.wav", 1);
    expect("HIT", new_single, 1);
    sound_manager_restore_snapshot();
    bank.resize(family_watermark);
    expect("HIT", reloaded, 3);
    check(sm.getCustomSoundId("HIT_0") == variant0, "family snapshot restored");
    const auto before_failure = bank.size();
    check(sound_manager_load_named_sound("RAW_ALIAS", "map01.wav", 33) == 0,
          "oversized raw alias family cannot silently truncate");
    check(bank.size() == before_failure, "oversized family adds no buffers");
    check(sound_manager_load_named_sound("RAW_ALIAS", "map01.wav", 3) == 0,
          "failed raw alias family returns failure to redirect caller");
    check(!sound_manager_is_registered("RAW_ALIAS") && !sound_manager_is_registered("RAW_ALIAS_0"),
          "failed new family publishes no registry entries");
    std::puts("SoundManager production registry and named loader tests passed");
}
