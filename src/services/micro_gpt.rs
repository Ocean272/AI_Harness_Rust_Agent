// use attention::{MicroGPT, generate};
// use candle_core::{DType, Device, Tensor};
// use candle_nn::VarBuilder;


 
//  let device = Device::Cpu;

//     let weights_path = "./models/micro_gpt.safetensors";

//     let vb = unsafe {
//         VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)?
//     };

//     let buffer = std::fs::read(weights_path)?;
//     let safetensors = safetensors::SafeTensors::deserialize(&buffer)?;
    
//     // for tensor_name in safetensors.names() {
//     //     println!("Tensor key: {}", tensor_name);
//     // }
//     // println!("Successfully loaded 'micro_gpt.safetensors' into Proxmox CPU memory!");

//     // Initialize MicroGPT using hyperparams matching your PyTorch model
//     let vocab_size = 65; // Adjust to your actual vocabulary size
//     let n_embd = 64;
//     let n_head = 4;
//     let n_layer = 4;
//     let block_size = 64;

//     let model = MicroGPT::new(vb, vocab_size, n_embd, n_head, n_layer)?;

//     // Prepare initial prompt/context tensor (e.g., single starting token ID 0)
//     let initial_tokens = vec![0u32];
//     let context = Tensor::from_slice(&initial_tokens, (1, 1), &device)?;

//     // Run autoregressive generation
//     let max_new_tokens = 50;
//     let generated_ids = generate(&model, context, max_new_tokens, block_size)?;

//     println!("Generated tokens IDs tensor shape: {:?}", generated_ids.shape());

//     // Define character lookup array (Adjust to match PyTorch training vocab string)
//     let vocab = " abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.\n";
//     let model_service = Arc::new(services::model::ModelService::new("./models/micro_gpt.safetensors", vocab)?);

//     let result_text = model_service.generate_text(vocab, max_new_tokens).await?;
//     println!("\n --- Decoded Result ---");
//     println!("{}", result_text);
