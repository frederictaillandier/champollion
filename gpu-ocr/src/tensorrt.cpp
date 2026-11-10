// A C interface to run a TensorRT engine with one input and one output,
// both float32, for the Rust side (src/engine.rs).

#include <NvInfer.h>
#include <cuda_runtime_api.h>

#include <cstdio>
#include <cstring>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

using namespace nvinfer1;

namespace {

class Logger : public ILogger {
    void log(Severity severity, const char* msg) noexcept override {
        if (severity <= Severity::kWARNING) {
            std::fprintf(stderr, "tensorrt: %s\n", msg);
        }
    }
};

Logger logger;

struct Engine {
    IRuntime* runtime = nullptr;
    ICudaEngine* engine = nullptr;
    IExecutionContext* context = nullptr;
    cudaStream_t stream = nullptr;
    std::string input;
    std::string output;
    void* input_buffer = nullptr;
    size_t input_capacity = 0;
    void* output_buffer = nullptr;
    size_t output_capacity = 0;
};

void set_error(char* error, size_t len, const std::string& message) {
    if (len == 0) return;
    std::strncpy(error, message.c_str(), len - 1);
    error[len - 1] = '\0';
}

size_t volume(const Dims& dims) {
    size_t n = 1;
    for (int i = 0; i < dims.nbDims; i++) n *= dims.d[i] < 0 ? 0 : dims.d[i];
    return n;
}

// Grows a device buffer to hold `bytes`.
bool reserve(void** buffer, size_t* capacity, size_t bytes) {
    if (bytes <= *capacity) return true;
    if (*buffer) cudaFree(*buffer);
    *buffer = nullptr;
    *capacity = 0;
    if (cudaMalloc(buffer, bytes) != cudaSuccess) return false;
    *capacity = bytes;
    return true;
}

void destroy(Engine* e) {
    if (e->input_buffer) cudaFree(e->input_buffer);
    if (e->output_buffer) cudaFree(e->output_buffer);
    if (e->stream) cudaStreamDestroy(e->stream);
    delete e->context;
    delete e->engine;
    delete e->runtime;
    delete e;
}

}  // namespace

extern "C" {

// Loads an engine file built by trtexec. Returns null and fills `error` on
// failure.
void* trt_open(const char* path, char* error, size_t error_len) {
    std::ifstream file(path, std::ios::binary);
    if (!file) {
        set_error(error, error_len, std::string("cannot read ") + path);
        return nullptr;
    }
    std::vector<char> data((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
    auto* e = new Engine();
    e->runtime = createInferRuntime(logger);
    e->engine = e->runtime ? e->runtime->deserializeCudaEngine(data.data(), data.size()) : nullptr;
    e->context = e->engine ? e->engine->createExecutionContext() : nullptr;
    if (!e->context) {
        destroy(e);
        set_error(error, error_len, std::string("cannot load the TensorRT engine ") + path);
        return nullptr;
    }
    for (int i = 0; i < e->engine->getNbIOTensors(); i++) {
        const char* name = e->engine->getIOTensorName(i);
        if (e->engine->getTensorIOMode(name) == TensorIOMode::kINPUT) {
            e->input = name;
        } else {
            e->output = name;
        }
    }
    if (e->engine->getNbIOTensors() != 2 || e->input.empty() || e->output.empty()) {
        destroy(e);
        set_error(error, error_len, "the engine must have one input and one output");
        return nullptr;
    }
    if (cudaStreamCreate(&e->stream) != cudaSuccess) {
        destroy(e);
        set_error(error, error_len, "cannot create a CUDA stream");
        return nullptr;
    }
    return e;
}

void trt_close(void* handle) {
    if (handle) destroy(static_cast<Engine*>(handle));
}

// Sets the input shape (`rank` sizes in `dims`) and writes the output shape
// it gives to `output_dims` (room for 8) and `output_rank`.
int trt_set_input_shape(void* handle, const int* dims, int rank, int* output_dims, int* output_rank,
                        char* error, size_t error_len) {
    auto* e = static_cast<Engine*>(handle);
    Dims shape;
    shape.nbDims = rank;
    for (int i = 0; i < rank; i++) shape.d[i] = dims[i];
    if (!e->context->setInputShape(e->input.c_str(), shape)) {
        set_error(error, error_len, "the input shape is outside what the engine was built for");
        return -1;
    }
    Dims out = e->context->getTensorShape(e->output.c_str());
    if (out.nbDims < 0 || out.nbDims > 8) {
        set_error(error, error_len, "unexpected output shape");
        return -1;
    }
    *output_rank = out.nbDims;
    for (int i = 0; i < out.nbDims; i++) output_dims[i] = out.d[i];
    return 0;
}

// Runs the engine on `input`, of the shape last set, and copies the result
// to `output`, which holds `output_len` floats.
int trt_run(void* handle, const float* input, size_t input_len, float* output, size_t output_len,
            char* error, size_t error_len) {
    auto* e = static_cast<Engine*>(handle);
    size_t in_bytes = input_len * sizeof(float);
    size_t out_needed = volume(e->context->getTensorShape(e->output.c_str()));
    if (input_len != volume(e->context->getTensorShape(e->input.c_str())) || output_len < out_needed) {
        set_error(error, error_len, "buffer sizes do not match the shapes");
        return -1;
    }
    size_t out_bytes = out_needed * sizeof(float);
    if (!reserve(&e->input_buffer, &e->input_capacity, in_bytes) ||
        !reserve(&e->output_buffer, &e->output_capacity, out_bytes)) {
        set_error(error, error_len, "out of GPU memory");
        return -1;
    }
    e->context->setTensorAddress(e->input.c_str(), e->input_buffer);
    e->context->setTensorAddress(e->output.c_str(), e->output_buffer);
    if (cudaMemcpyAsync(e->input_buffer, input, in_bytes, cudaMemcpyHostToDevice, e->stream) != cudaSuccess ||
        !e->context->enqueueV3(e->stream) ||
        cudaMemcpyAsync(output, e->output_buffer, out_bytes, cudaMemcpyDeviceToHost, e->stream) != cudaSuccess ||
        cudaStreamSynchronize(e->stream) != cudaSuccess) {
        set_error(error, error_len, "inference failed");
        return -1;
    }
    return 0;
}

}  // extern "C"
