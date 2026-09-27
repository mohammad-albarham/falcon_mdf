// Writes Ethernet and FlexRay bus-logging MF4 files with ihedvall/mdflib, an
// independent C++ implementation, so falcon_mdf's frame readers are checked
// against files this crate did not write. Every field follows a formula the
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

}  // namespace

int main(int argc, char** argv) {
  if (argc != 3) {
    std::fprintf(stderr, "usage: %s <ethernet.mf4> <flexray.mf4>\n", argv[0]);
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
  return 0;
}
