// Writes Ethernet and FlexRay bus-logging MF4 files, and one with big-endian
// channels, with ihedvall/mdflib, an independent C++ implementation, so
// falcon_mdf's readers are checked against files this crate did not write. Every field follows a formula the
// Rust test (tests/bus_mdflib_fixtures.rs) recomputes; nothing is read back
// from the files to build the expectation.
//
// Built and run by scripts/make_mdflib_bus_fixtures.sh.

#include <array>
#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

#include "mdf/ethconfigadapter.h"
#include "mdf/ethmessage.h"
#include "mdf/flexrayconfigadapter.h"
#include "mdf/flexraymessage.h"
#include "mdf/ichannel.h"
#include "mdf/ichannelarray.h"
#include "mdf/ichannelgroup.h"
#include "mdf/idatagroup.h"
#include "mdf/ifilehistory.h"
#include "mdf/mdffactory.h"
#include "mdf/mdfwriter.h"

using namespace mdf;

namespace {

constexpr size_t kFrames = 24;
constexpr uint64_t kStartNs = 1'700'000'000'000'000'000ULL;
constexpr uint64_t kStepNs = 1'000'000;  // 1 ms

bool WriteEthernet(const std::string& path) {
  auto writer = MdfFactory::CreateMdfWriter(MdfWriterType::MdfBusLogger);
  if (!writer || !writer->Init(path)) return false;
  auto* history = writer->Header()->CreateFileHistory();
  history->Description("falcon_mdf Ethernet fixture");
  history->ToolName("mdflib_bus_fixtures");
  history->ToolVendor("falcon_mdf");
  history->ToolVersion("1.0");
  writer->BusType(MdfBusType::Ethernet);
  writer->StorageType(MdfStorageType::VlsdStorage);
  writer->PreTrigTime(0.0);
  writer->CompressData(false);
  if (!writer->CreateBusLogConfiguration()) return false;
  auto* dg = writer->Header()->LastDataGroup();
  IChannelGroup* frames = dg->GetChannelGroup("ETH_Frame");
  if (frames == nullptr) return false;
  if (!writer->InitMeasurement()) return false;
  uint64_t t = kStartNs;
  writer->StartMeasurement(t);
  for (size_t i = 0; i < kFrames; ++i) {
    // Payload i: (i % 9) + 1 bytes, byte k = (i * 7 + k) & 0xFF.
    std::vector<uint8_t> data((i % 9) + 1);
    for (size_t k = 0; k < data.size(); ++k) data[k] = static_cast<uint8_t>((i * 7 + k) & 0xFF);
    const std::array<uint8_t, 6> src = {0x02, 0x00, 0x00, 0x00, 0x00, static_cast<uint8_t>(i)};
    const std::array<uint8_t, 6> dst = {0x02, 0xFF, 0x00, 0x00, 0x00, static_cast<uint8_t>(i)};
    EthMessage msg;
    msg.BusChannel(static_cast<uint8_t>(i % 3 + 1));
    msg.Dir(i % 2 == 0);
    msg.Source(src);
    msg.Destination(dst);
    msg.EthType(static_cast<uint16_t>(0x0800 + i));
    msg.ReceivedDataByteCount(static_cast<uint16_t>(data.size()));
    msg.DataBytes(data);
    writer->SaveEthMessage(*frames, t, msg);
    t += kStepNs;
  }
  writer->StopMeasurement(t);
  return writer->FinalizeMeasurement();
}

bool WriteFlexRay(const std::string& path) {
  auto writer = MdfFactory::CreateMdfWriter(MdfWriterType::MdfBusLogger);
  if (!writer || !writer->Init(path)) return false;
  auto* history = writer->Header()->CreateFileHistory();
  history->Description("falcon_mdf FlexRay fixture");
  history->ToolName("mdflib_bus_fixtures");
  history->ToolVendor("falcon_mdf");
  history->ToolVersion("1.0");
  writer->BusType(MdfBusType::FlexRay);
  writer->StorageType(MdfStorageType::VlsdStorage);
  writer->PreTrigTime(0.0);
  writer->CompressData(false);
  auto* dg = writer->Header()->CreateDataGroup();
  FlexRayConfigAdapter config(*writer);
  config.CreateConfig(*dg);
  IChannelGroup* frames = dg->GetChannelGroup("FLX_Frame");
  if (frames == nullptr) return false;
  if (!writer->InitMeasurement()) return false;
  uint64_t t = kStartNs;
  writer->StartMeasurement(t);
  for (size_t i = 0; i < kFrames; ++i) {
    // Payload i: 2 * (i % 5) bytes (FlexRay payloads are whole 16-bit words),
    // byte k = (i * 5 + k) & 0xFF.
    std::vector<uint8_t> data(2 * (i % 5));
    for (size_t k = 0; k < data.size(); ++k) data[k] = static_cast<uint8_t>((i * 5 + k) & 0xFF);
    FlexRayFrame frame;
    frame.BusChannel(static_cast<uint8_t>(i % 2 + 1));
    frame.FrameId(static_cast<uint16_t>(100 + i));
    frame.CycleCount(static_cast<uint8_t>((i * 3) & 0x3F));
    frame.Direction(i % 2 == 0 ? FlexRayDirection::Tx : FlexRayDirection::Rx);
    frame.PayloadLength(static_cast<uint8_t>(data.size() / 2));
    frame.DataBytes(data);
    // Every sixth frame is a null frame; FlexRay's indicator is 0 for those.
    frame.NullFrame(i % 6 == 5 ? FlexRayNullFlag::NullFrame : FlexRayNullFlag::NormalFrame);
    frame.SyncFrame(i % 4 == 0);
    frame.StartupFrame(i % 8 == 0);
    writer->SaveFlexRayMessage(*dg, *frames, t, frame);
    t += kStepNs;
  }
  writer->StopMeasurement(t);
  return writer->FinalizeMeasurement();
}

IChannel* AddChannel(IChannelGroup& group, const char* name, ChannelDataType type,
                     uint32_t bytes) {
  auto* ch = group.CreateChannel();
  ch->Name(name);
  ch->Type(ChannelType::FixedLength);
  ch->Sync(ChannelSyncType::None);
  ch->DataType(type);
  ch->DataBytes(bytes);
  return ch;
}

// Big-endian ("Motorola") channels of every numeric kind, beside one
// little-endian twin, so a reader that ignores the byte order fails loudly.
bool WriteBigEndian(const std::string& path) {
  constexpr size_t kSamples = 50;
  auto writer = MdfFactory::CreateMdfWriter(MdfWriterType::Mdf4Basic);
  if (!writer || !writer->Init(path)) return false;
  auto* header = writer->Header();
  auto* history = header->CreateFileHistory();
  history->Description("falcon_mdf big-endian fixture");
  history->ToolName("mdflib_bus_fixtures");
  history->ToolVendor("falcon_mdf");
  history->ToolVersion("1.0");
  auto* dg = header->CreateDataGroup();
  auto* cg = dg->CreateChannelGroup();
  cg->Name("Motorola");
  auto* time = cg->CreateChannel();
  time->Name("Time");
  time->Type(ChannelType::Master);
  time->Sync(ChannelSyncType::Time);
  time->DataType(ChannelDataType::FloatLe);
  time->DataBytes(8);
  time->Unit("s");
  auto* u16le = AddChannel(*cg, "U16_Le", ChannelDataType::UnsignedIntegerLe, 2);
  auto* u16be = AddChannel(*cg, "U16_Be", ChannelDataType::UnsignedIntegerBe, 2);
  auto* i32be = AddChannel(*cg, "I32_Be", ChannelDataType::SignedIntegerBe, 4);
  auto* u64be = AddChannel(*cg, "U64_Be", ChannelDataType::UnsignedIntegerBe, 8);
  auto* f32be = AddChannel(*cg, "F32_Be", ChannelDataType::FloatBe, 4);
  auto* f64be = AddChannel(*cg, "F64_Be", ChannelDataType::FloatBe, 8);
  writer->PreTrigTime(0);
  if (!writer->InitMeasurement()) return false;
  uint64_t t = kStartNs;
  writer->StartMeasurement(t);
  for (size_t i = 0; i < kSamples; ++i) {
    const auto n = static_cast<int64_t>(i);
    u16le->SetChannelValue(static_cast<uint64_t>((i * 1000 + 7) & 0xFFFF));
    u16be->SetChannelValue(static_cast<uint64_t>((i * 1000 + 7) & 0xFFFF));
    i32be->SetChannelValue(-n * 100'003 + 17);
    u64be->SetChannelValue(static_cast<uint64_t>(0x0102030405060000ULL + i));
    f32be->SetChannelValue(static_cast<double>(i) * 1.5 - 20.25);
    f64be->SetChannelValue(static_cast<double>(i) * 0.125 - 3.0);
    writer->SaveSample(*cg, t);
    t += kStepNs;
  }
  writer->StopMeasurement(t);
  return writer->FinalizeMeasurement();
}

// Mixed content for general conformance: integers and floats in both byte
// orders, a channel with invalidation bits, and variable-length strings (UTF-8
// with non-ASCII characters, and ASCII). 5,000 samples, so the data spans
// several blocks; written once plain and once through mdflib's ##DZ path.
bool WriteMixed(const std::string& path, bool compress) {
  constexpr size_t kSamples = 5000;
  auto writer = MdfFactory::CreateMdfWriter(MdfWriterType::Mdf4Basic);
  if (!writer || !writer->Init(path)) return false;
  auto* header = writer->Header();
  auto* history = header->CreateFileHistory();
  history->Description(compress ? "falcon_mdf mixed fixture, compressed"
                                : "falcon_mdf mixed fixture");
  history->ToolName("mdflib_bus_fixtures");
  history->ToolVendor("falcon_mdf");
  history->ToolVersion("1.0");
  writer->CompressData(compress);
  auto* dg = header->CreateDataGroup();
  auto* cg = dg->CreateChannelGroup();
  cg->Name("Mixed");
  auto* time = cg->CreateChannel();
  time->Name("Time");
  time->Type(ChannelType::Master);
  time->Sync(ChannelSyncType::Time);
  time->DataType(ChannelDataType::FloatLe);
  time->DataBytes(8);
  time->Unit("s");
  auto* u32 = AddChannel(*cg, "U32", ChannelDataType::UnsignedIntegerLe, 4);
  auto* i16be = AddChannel(*cg, "I16_Be", ChannelDataType::SignedIntegerBe, 2);
  auto* f64 = AddChannel(*cg, "F64", ChannelDataType::FloatLe, 8);
  auto* gappy = AddChannel(*cg, "F32_Invalid", ChannelDataType::FloatLe, 4);
  gappy->Flags(CnFlag::InvalidValid);
  auto* utf8 = cg->CreateChannel();
  utf8->Name("Text_Utf8");
  utf8->Type(ChannelType::VariableLength);
  utf8->Sync(ChannelSyncType::None);
  utf8->DataType(ChannelDataType::StringUTF8);
  utf8->DataBytes(8);
  auto* ascii = cg->CreateChannel();
  ascii->Name("Text_Ascii");
  ascii->Type(ChannelType::VariableLength);
  ascii->Sync(ChannelSyncType::None);
  ascii->DataType(ChannelDataType::StringAscii);
  ascii->DataBytes(8);
  writer->PreTrigTime(0);
  if (!writer->InitMeasurement()) return false;
  uint64_t t = kStartNs;
  writer->StartMeasurement(t);
  for (size_t i = 0; i < kSamples; ++i) {
    const auto n = static_cast<int64_t>(i);
    u32->SetChannelValue(static_cast<uint64_t>(i * 7919));
    i16be->SetChannelValue(-(n % 1000));
    f64->SetChannelValue(static_cast<double>(i) * 0.001 - 1.0);
    gappy->SetChannelValue(static_cast<double>(i) * 0.5, i % 7 != 0);
    std::string text = "sample-" + std::to_string(i);
    if (i % 3 == 0) text += "-gr\xC3\xBC\xC3\x9F";  // "-grüß" in UTF-8
    utf8->SetChannelValue(text);
    ascii->SetChannelValue(std::string(i % 5, 'x') + std::to_string(i));
    writer->SaveSample(*cg, t);
    t += kStepNs;
  }
  writer->StopMeasurement(t);
  return writer->FinalizeMeasurement();
}

// A CN-template array channel: shape 3x4 f64, element `index` of sample `s`
// holding s * 100 + index, where `index` is mdflib's linear array index.
bool WriteArray(const std::string& path) {
  constexpr size_t kSamples = 10;
  auto writer = MdfFactory::CreateMdfWriter(MdfWriterType::Mdf4Basic);
  if (!writer || !writer->Init(path)) return false;
  auto* header = writer->Header();
  auto* dg = header->CreateDataGroup();
  auto* cg = dg->CreateChannelGroup();
  cg->Name("Arrays");
  auto* time = cg->CreateChannel();
  time->Name("Time");
  time->Type(ChannelType::Master);
  time->Sync(ChannelSyncType::Time);
  time->DataType(ChannelDataType::FloatLe);
  time->DataBytes(8);
  auto* matrix = AddChannel(*cg, "Matrix", ChannelDataType::FloatLe, 8);
  auto* array = matrix->CreateChannelArray();
  array->Type(ArrayType::Array);
  array->Shape({3, 4});
  const uint64_t elements = array->NofArrayValues();
  writer->PreTrigTime(0);
  if (!writer->InitMeasurement()) return false;
  uint64_t t = kStartNs;
  writer->StartMeasurement(t);
  for (size_t s = 0; s < kSamples; ++s) {
    for (uint64_t index = 0; index < elements; ++index) {
      matrix->SetChannelValue(static_cast<double>(s * 100 + index), true, index);
    }
    writer->SaveSample(*cg, t);
    t += kStepNs;
  }
  writer->StopMeasurement(t);
  return writer->FinalizeMeasurement();
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 7) {
    std::fprintf(stderr,
                 "usage: %s <ethernet.mf4> <flexray.mf4> <big_endian.mf4> <mixed.mf4> "
                 "<mixed_compressed.mf4> <array.mf4>\n",
                 argv[0]);
    return 2;
  }
  if (!WriteEthernet(argv[1])) {
    std::fprintf(stderr, "failed to write %s\n", argv[1]);
    return 1;
  }
  if (!WriteFlexRay(argv[2])) {
    std::fprintf(stderr, "failed to write %s\n", argv[2]);
    return 1;
  }
  if (!WriteBigEndian(argv[3])) {
    std::fprintf(stderr, "failed to write %s\n", argv[3]);
    return 1;
  }
  if (!WriteMixed(argv[4], false) || !WriteMixed(argv[5], true)) {
    std::fprintf(stderr, "failed to write the mixed fixtures\n");
    return 1;
  }
  if (!WriteArray(argv[6])) {
    std::fprintf(stderr, "failed to write %s\n", argv[6]);
    return 1;
  }
  return 0;
}
