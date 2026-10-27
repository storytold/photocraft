using System.Text.Json;
using PhotoCraft.OnnxQnn;

namespace PhotoCraft.OnnxQnn.Tests;

internal static partial class Program
{
    private sealed class FakeBackend : IInferenceBackend
    {
        internal int Checks, Creates, Runs, Disposals;
        internal bool FailPayload, FailCreate, FailRun, WrongBinding;
        public void CheckPayload(string root) { Checks++; if (FailPayload) throw new WorkerException("FAKE missing payload"); }
        public IInferenceSession Create(string root, VerifiedModel model)
        {
            Creates++;
            if (FailCreate) throw new WorkerException("FAKE strict-session failure");
            return new FakeSession(this, model.Profile);
        }
    }
    private sealed class FakeSession(FakeBackend backend, ModelProfile profile) : IInferenceSession
    {
        public TensorBinding[] Inputs => [new(profile.input.name, backend.WrongBinding ? "uint8" : "float32", profile.input.shape)];
        public TensorBinding[] Outputs => [new(profile.output.name, "float32", profile.output.shape)];
        public TensorData Run(string inputName, string outputName, TensorData input)
        {
            backend.Runs++;
            if (backend.FailRun) throw new WorkerException("FAKE inference failure");
            Check(inputName == profile.input.name && outputName == profile.output.name);
            return new([0.5001f, 0.5002f, 0.001f, 1f], (int[])profile.output.shape.Clone());
        }
        public void Dispose() { backend.Disposals++; }
    }
    private static void TestProfilesAndWorker()
    {
        Run("paths/traversal-absolute-ads-unc-hidden-device-duplicates", () =>
        {
            foreach (string name in new[] { "../x.onnx", "a/../x.onnx", "/x.onnx", "C:/x.onnx", "x:stream.onnx", "\\\\server\\x.onnx", "a\\x.onnx", ".hidden/x.onnx", "a//b.onnx", "CON.onnx", "LPT1.onnx", "a./x.onnx", new string('a', 129) + ".onnx" })
                Reject<WorkerException>(() => Profiles.Validate(Profile() with { model = name }));
            Temp(root =>
            {
                var p = Profile(); Registry(root, new(1, p.id, [p, p with { id = "other", model = "MODEL.onnx" }]));
                Reject<WorkerException>(() => Profiles.Load(root));
            });
        });
        Run("profiles/registry-required-fields-null-unknown-and-duplicates", () => Temp(root =>
        {
            foreach (string json in new[] { "{}", "null", "{\"schemaVersion\":1,\"defaultModel\":null,\"profiles\":null}",
                "{\"schemaVersion\":1,\"defaultModel\":null,\"profiles\":[],\"extra\":0}",
                "{\"schemaVersion\":1,\"schemaVersion\":1,\"defaultModel\":null,\"profiles\":[]}",
                "{\"schemaVersion\":1,\"defaultModel\":null,\"profiles\":[null]}" })
            {
                File.WriteAllText(Path.Combine(root, Profiles.ConfigName), json);
                Reject<WorkerException>(() => Profiles.Load(root));
            }
            var profile = Profile();
            string text = JsonSerializer.Serialize(new WorkerConfig(1, profile.id, [profile]), Json.Options);
            File.WriteAllText(Path.Combine(root, Profiles.ConfigName), text.Replace("\"resize\":\"nearest\",", ""));
            Reject<WorkerException>(() => Profiles.Load(root));
        }));
        Run("profiles/shape-layout-scalar-only-and-normalization", () =>
        {
            var p = Profile(); Profiles.Validate(p);
            foreach (var bad in new[] { p with { input = p.input with { shape = [1, 3, -1, 2] } },
                p with { input = p.input with { shape = [2, 3, 2, 2] } }, p with { input = p.input with { shape = [1, 3, 4096, 4096] } },
                p with { input = p.input with { layout = "unknown" } }, p with { input = p.input with { scale = [1, 0, 1] } },
                p with { input = p.input with { mean = [0, float.NaN, 0] } }, p with { input = p.input with { alphaPolicy = "premultiplied" } },
                p with { output = p.output with { shape = [1, 2, 2, 2] } }, p with { output = p.output with { postprocess = "softmax" } },
                p with { sha256 = "fabricated" } }) Reject<WorkerException>(() => Profiles.Validate(bad));
        });
        Run("profiles/runtime-binding-name-dtype-static-shape-cardinality", () =>
        {
            var p = Profile(); TensorBinding input = new("input", "float32", p.input.shape), output = new("output", "float32", p.output.shape);
            Profiles.Bindings(p, [input], [output]);
            foreach (var bad in new[] { input with { Name = "other" }, input with { ElementType = "uint8" }, input with { Shape = [-1, 3, 2, 2] } })
                Reject<WorkerException>(() => Profiles.Bindings(p, [bad], [output]));
            Reject<WorkerException>(() => Profiles.Bindings(p, [input, input], [output]));
            Reject<WorkerException>(() => Profiles.Bindings(p, [input], [output with { Shape = [1, 2, 2, 2] }]));
        });
        Run("profiles/digest-missing-external-tensor-before-backend", () => Temp(root =>
        {
            var p = InstallSynthetic(root); var profiles = Profiles.Load(root);
            using (profiles.Verify(p)) { }
            Reject<WorkerException>(() => profiles.Verify(p with { sha256 = new string('0', 64) }));
            File.WriteAllBytes(Path.Combine(root, "model.onnx"), Model(Graph(Field(5, Int(14, 1)))));
            string hash = Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(File.ReadAllBytes(Path.Combine(root, "model.onnx"))));
            Reject<WorkerException>(() => profiles.Verify(p with { sha256 = hash }));
            File.Delete(Path.Combine(root, "model.onnx"));
            Reject<NeedsModelException>(() => profiles.Verify(p));
        }));
        Run("payload/all-actual-native-dsp-files-required-no-native-loading", () => Temp(root =>
        {
            Reject<WorkerException>(() => Payload.Check(root));
            foreach (string name in Payload.RuntimeNames) File.WriteAllBytes(Path.Combine(root, name), [1]);
            Payload.Check(root); // Existence-only fake files, never passed to ORT.
            foreach (string name in Payload.RuntimeNames)
            {
                File.WriteAllBytes(Path.Combine(root, name), []);
                Reject<WorkerException>(() => Payload.Check(root));
                File.WriteAllBytes(Path.Combine(root, name), [1]);
            }
        }));
        Run("worker/no-model-before-native-or-architecture-probe-and-prepare", () => Temp(root =>
        {
            foreach (string action in new[] { "probe", "prepare" })
            foreach (bool arm in new[] { false, true })
            {
                var backend = new FakeBackend { FailPayload = true };
                var reply = Worker.Execute(action, null, root, arm, backend);
                Check(reply.Header.status == "needs_model" && backend.Checks == 0 && backend.Creates == 0);
            }
            Registry(root, new(1, null, []));
            Check(Worker.Execute("probe", null, root, false, new FakeBackend()).Header.status == "needs_model");
        }));
        Run("worker/configured-model-non-arm64-unavailable-no-session", () => Temp(root =>
        {
            InstallSynthetic(root); var backend = new FakeBackend();
            Check(Worker.Execute("prepare", null, root, false, backend).Header.status == "unavailable");
            Check(backend.Checks == 0 && backend.Creates == 0);
        }));
        Run("worker/fake-probe-no-session-prepare-bindings-and-disposal", () => Temp(root =>
        {
            InstallSynthetic(root); var backend = new FakeBackend();
            Check(Worker.Execute("probe", null, root, true, backend).Header.status == "ready");
            Check(backend.Creates == 0 && backend.Runs == 0);
            Check(Worker.Execute("prepare", null, root, true, backend).Header.status == "ready");
            Check(backend.Creates == 1 && backend.Runs == 0 && backend.Disposals == 1);
        }));
        Run("worker/fake-run-authoritative-f32-and-strict-error-no-retry", () => Temp(root =>
        {
            InstallSynthetic(root); var backend = new FakeBackend();
            var reply = Worker.Execute("run", Image(), root, true, backend);
            Check(reply.Header.status == "ok" && backend.Runs == 1 && backend.Disposals == 1);
            Near(0.5001f, reply.Mask![0]); Near(0.5002f, reply.Mask[1]);
            Check(reply.Header.execution?.backend == "qnn"); // Fake scheduling assertion, not NPU evidence.
            foreach (var broken in new[] { new FakeBackend { FailPayload = true }, new FakeBackend { FailCreate = true }, new FakeBackend { FailRun = true }, new FakeBackend { WrongBinding = true } })
            {
                Reject<WorkerException>(() => Worker.Execute("run", Image(), root, true, broken));
                Check(broken.Creates <= 1 && broken.Runs <= 1);
                if (broken.FailRun || broken.WrongBinding) Check(broken.Disposals == 1);
            }
        }));
    }
    private static void TestImaging()
    {
        Run("imaging/rgb-layout-alpha-normalization-values", () =>
        {
            var p = Profile();
            var nchw = Imaging.Preprocess(Image(alpha: true), p);
            Check(nchw.Samples.SequenceEqual(new[] { 0f, 0, 0, 0, .5f, .5f, .5f, .5f, 1, 1, 1, 1 }));
            var nhwc = Imaging.Preprocess(Image(), p with { input = p.input with { layout = "nhwc", shape = [1, 2, 2, 3] } });
            Check(nhwc.Samples.SequenceEqual(Image().Samples));
            var normalized = Imaging.Preprocess(Image(), p with { input = p.input with { mean = [.5f, .5f, .5f], scale = [2, 2, 2] } });
            Near(-1, normalized.Samples[0]); Near(0, normalized.Samples[4]); Near(1, normalized.Samples[8]);
        });
        Run("imaging/bilinear-pixel-centers-and-no-premultiply", () =>
        {
            var p = Profile(); var image = Image();
            float[] source = [0, 0, 0, 1, 1, 1, 0, 0, 0, 1, 1, 1];
            var tensor = Imaging.Preprocess(image with { Samples = source }, p with { input = p.input with { shape = [1, 3, 1, 1], resize = "bilinear" } });
            Check(tensor.Samples.SequenceEqual(new[] { .5f, .5f, .5f }));
            var mask = Imaging.Postprocess(new([0, 1, 0, 1], [1, 1, 2, 2]), p with { output = p.output with { resize = "bilinear" } }, Metadata() with { width = 1, height = 1 });
            Near(.5f, mask[0]);
        });
        Run("imaging/probability-sigmoid-range-nonfinite-and-shape", () =>
        {
            var p = Profile();
            var mask = Imaging.Postprocess(new([float.MinValue, 0, float.MaxValue, -10], [1, 1, 2, 2]), p with { output = p.output with { postprocess = "sigmoid" } }, Metadata());
            Near(0, mask[0]); Near(.5f, mask[1]); Near(1, mask[2]); Check(mask[3] < .001f);
            foreach (float v in new[] { float.NaN, float.PositiveInfinity, -0.1f, 1.1f })
                Reject<WorkerException>(() => Imaging.Postprocess(new([v, 0, 0, 0], [1, 1, 2, 2]), p, Metadata()));
            Reject<WorkerException>(() => Imaging.Postprocess(new([0, 0, 0, 0], [1, 2, 2, 1]), p, Metadata()));
            Reject<WorkerException>(() => Imaging.Preprocess(Image() with { Samples = [0] }, p));
        });
    }
}
