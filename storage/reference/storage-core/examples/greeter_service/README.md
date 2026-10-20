# Greeter Service Example

This example shows the full process of adding an entirely new service to use alongside the built in services provided by the storage-core library. 

The steps for creating a new service are as follows:

## Define and Compile the Proto file
The first step to creating a service is to define the proto file and compile it. In this example, we're using the basic (`helloworld.proto`)[https://github.com/hyperium/tonic/blob/master/examples/proto/helloworld/helloworld.proto]. The proto file is at `proto/helloworld.proto`. It's compiled in `build.rs`, and imported in `helloworld/proto.rs`. 

## Define the Trait the Represents the Service
Defining a common trait that represents the Service simplifies what implementers need to implement, and allows a single implementation to be used for GRPC and REST. Here the trait is defined in `helloworld/traits.rs`

## Implement a Backend for the Trait
All Services require a backend implemenation that has concrete implementation of the Service methods (or retunrs unimplemented). Here that backend is implemented in `helloworld/backend.rs`.

## Implement GRPC and Rest Ingresses using the Trait
Ingresses are the entry points for the service. They are responsible for receiving requests from the client and forwarding them to the backend. Here the GRPC and REST ingresses are implemented in `helloworld/ingress.rs`.

## Implement a Builder for the Service
The builder is responsible for creating a new instance of the service and adding layers to it. It's implemented in `helloworld/builder.rs`.

## Implement any Additional Layers for the Service
You could also implement additional layers for the Service. The layer should implement the trait you defined for the Service, and can also implement additional traits from the storage-core library. In this example, there is a `stat_layer` layer that stores the last stat operation and adds it to the response of the Greeter Service.

## Building

### Build Commands

There are two feature flags for this example: `azure` and `s3`. `azure` is the default feature, and `s3` can be selected with a feature flag:

Build with Azure backend:
```bash
cargo build
```

Build with S3 backend:
```bash
cargo build --features s3 --no-default-features
```

**Note**: You must specify either the `azure` or `s3` feature - both cannot be enabled simultaneously.

## Running

### Basic Usage

Run with default settings:
```bash
# For Azure backend
cargo run

# For S3 backend  
cargo run --features s3 --no-default-features
```

### Command Line Options

- `--grpc-addr`: gRPC server address (default: 0.0.0.0:50051)
- `--http-addr`: HTTP server address (default: 0.0.0.0:8011)
- `--config`: Path to configuration file
