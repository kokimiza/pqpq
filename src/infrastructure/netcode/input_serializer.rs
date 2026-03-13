use crate::domain::services::InputSerializer as InputSerializerTrait;
use crate::domain::value_objects::CombatInputKey;
use anyhow::{Result, anyhow};

/// 入力データのシリアライズ/デシリアライズ
///
/// WebRTC DataChannelで送信するための軽量なバイナリフォーマット
/// フォーマット: [frame: u32][input: u8]
pub struct InputSerializerImpl;

impl InputSerializerImpl {
    pub fn new() -> Self {
        Self
    }
}

impl Default for InputSerializerImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl InputSerializerTrait for InputSerializerImpl {
    fn serialize(&self, frame: u32, input: CombatInputKey) -> Vec<u8> {
        let mut data = Vec::with_capacity(5);
        data.extend_from_slice(&frame.to_le_bytes());
        data.push(input.as_byte());
        data
    }

    fn deserialize(&self, data: &[u8]) -> Result<(u32, CombatInputKey)> {
        if data.len() < 5 {
            return Err(anyhow!("Invalid input data length: {}", data.len()));
        }

        let frame = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let input = CombatInputKey::from_byte(data[4]);

        Ok((frame, input))
    }

    fn serialize_batch(&self, inputs: &[(u32, CombatInputKey)]) -> Vec<u8> {
        let mut data = Vec::with_capacity(inputs.len() * 5 + 2);

        // バッチサイズを先頭に追加
        data.extend_from_slice(&(inputs.len() as u16).to_le_bytes());

        for (frame, input) in inputs {
            data.extend_from_slice(&frame.to_le_bytes());
            data.push(input.as_byte());
        }

        data
    }

    fn deserialize_batch(&self, data: &[u8]) -> Result<Vec<(u32, CombatInputKey)>> {
        if data.len() < 2 {
            return Err(anyhow!("Invalid batch data length: {}", data.len()));
        }

        let batch_size = u16::from_le_bytes([data[0], data[1]]) as usize;
        let expected_len = 2 + batch_size * 5;

        if data.len() < expected_len {
            return Err(anyhow!(
                "Invalid batch data length: expected {}, got {}",
                expected_len,
                data.len()
            ));
        }

        let mut inputs = Vec::with_capacity(batch_size);
        let mut offset = 2;

        for _ in 0..batch_size {
            let frame = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            let input = CombatInputKey::from_byte(data[offset + 4]);
            inputs.push((frame, input));
            offset += 5;
        }

        Ok(inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_deserialize() {
        let serializer = InputSerializerImpl::new();
        let frame = 42;
        let mut input = CombatInputKey::empty();
        input.press(CombatInputKey::UP);
        input.press(CombatInputKey::PUNCH);

        let data = serializer.serialize(frame, input);
        let (decoded_frame, decoded_input) = serializer.deserialize(&data).unwrap();

        assert_eq!(frame, decoded_frame);
        assert_eq!(input.as_byte(), decoded_input.as_byte());
    }

    #[test]
    fn test_serialize_batch() {
        let serializer = InputSerializerImpl::new();

        let mut input1 = CombatInputKey::empty();
        input1.press(CombatInputKey::UP);

        let mut input2 = CombatInputKey::empty();
        input2.press(CombatInputKey::DOWN);
        input2.press(CombatInputKey::KICK);

        let input3 = CombatInputKey::empty();

        let inputs = vec![(10, input1), (11, input2), (12, input3)];

        let data = serializer.serialize_batch(&inputs);
        let decoded = serializer.deserialize_batch(&data).unwrap();

        assert_eq!(inputs.len(), decoded.len());
        for (i, (frame, input)) in inputs.iter().enumerate() {
            assert_eq!(*frame, decoded[i].0);
            assert_eq!(input.as_byte(), decoded[i].1.as_byte());
        }
    }
}
