use tch::{Tensor, nn};
use crate::distilbert::dropout::Dropout;
use crate::distilbert::distilbert::{DistilBertConfig, Activation};
use crate::distilbert::attention::MultiHeadSelfAttention;
use tch::nn::LayerNorm;
use std::borrow::BorrowMut;

fn _gelu(x: &Tensor) -> Tensor {
    x * 0.5 * (1.0 + (x / ((2.0 as f64).sqrt())).erf())
}

fn _relu(x: &Tensor) -> Tensor {
    x.relu()
}

pub struct FeedForwardNetwork {
    lin1: nn::Linear,
    lin2: nn::Linear,
    dropout: Dropout,
    activation: Box<dyn Fn(&Tensor) -> Tensor>,
}

impl FeedForwardNetwork {
    pub fn new(p: nn::Path, config: &DistilBertConfig) -> FeedForwardNetwork {
        let lin1 = nn::linear(&p / "lin1", config.dim, config.hidden_dim, Default::default());
        let lin2 = nn::linear(&p / "lin2", config.hidden_dim, config.dim, Default::default());
        let dropout = Dropout::new(config.dropout);
        let activation = Box::new(match &config.activation {
            Activation::Gelu => _gelu,
            Activation::Relu => _relu
        });
        FeedForwardNetwork { lin1, lin2, dropout, activation }
    }

    pub fn forward_t(&self, input: &Tensor, train: bool) -> Tensor {
        (self.activation)(&input.apply(&self.lin1)).apply(&self.lin2).apply_t(&self.dropout, train)
    }
}

pub struct TransformerBlock {
    attention: MultiHeadSelfAttention,
    sa_layer_norm: LayerNorm,
    ffn: FeedForwardNetwork,
    output_layer_norm: LayerNorm,
}

impl TransformerBlock {
    pub fn new(p: nn::Path, config: &DistilBertConfig) -> TransformerBlock {
        let attention = MultiHeadSelfAttention::new(&p / "attention", &config);
        let layer_norm_config = nn::LayerNormConfig { eps: 1e-12, ..Default::default() };
        let sa_layer_norm = nn::layer_norm(&p, vec![config.dim], layer_norm_config);
        let ffn = FeedForwardNetwork::new(&p / "FFN", &config);
        let output_layer_norm = nn::layer_norm(&p, vec![config.dim], layer_norm_config);

        TransformerBlock {
            attention,
            sa_layer_norm,
            ffn,
            output_layer_norm,
        }
    }

    pub fn forward_t(&self, input: &Tensor, mask: Option<&Tensor>, train: bool) -> (Tensor, Option<Tensor>) {
        let (output, sa_weights) = self.attention.forward_t(&input, &input, &input, mask, train);
        let output = (input + &output).apply(&self.sa_layer_norm);
        let output = (&output + self.ffn.forward_t(&output, train)).apply(&self.output_layer_norm);
        (output, sa_weights)
    }
}

pub struct Transformer {
    output_attentions: bool,
    output_hidden_states: bool,
    layers: Vec<TransformerBlock>,
}

impl Transformer {
    pub fn new(p: nn::Path, config: &DistilBertConfig) -> Transformer {
        let output_attentions = config.output_attentions;
        let output_hidden_states = config.output_hidden_states;

        let mut layers: Vec<TransformerBlock> = vec!();
        for layer_index in 0..config.n_layers {
            layers.push(TransformerBlock::new(&p / layer_index, config));
        };

        Transformer { output_attentions, output_hidden_states, layers }
    }

    pub fn forward_t(&self, input: &Tensor, mask: Option<&Tensor>, train: bool)
                     -> (Tensor, Option<Vec<Tensor>>, Option<Vec<Tensor>>) {
        let mut all_hidden_states: Option<Vec<Tensor>> = if self.output_hidden_states { Some(vec!()) } else { None };
        let mut all_attentions: Option<Vec<Tensor>> = if self.output_attentions { Some(vec!()) } else { None };

        let mut hidden_state = input.copy();
        let mut attention_weights: Option<Tensor>;
        let mut layers = self.layers.iter();
        loop {
            match layers.next() {
                Some(layer) => {
                    if let Some(hidden_states) = all_hidden_states.borrow_mut() {
                        hidden_states.push(hidden_state.as_ref().copy());
                    };

                    let temp = layer.forward_t(&hidden_state, mask, train);
                    hidden_state = temp.0;
                    attention_weights = temp.1;
                    if let Some(attentions) = all_attentions.borrow_mut() {
                        attentions.push(attention_weights.as_ref().unwrap().copy());
                    };
                }
                None => break
            };
        };

        (hidden_state, all_hidden_states, all_attentions)
    }
}