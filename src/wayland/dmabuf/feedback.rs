use waynest::{
	server::{
		Client, Dispatcher, Result,
		protocol::stable::linux_dmabuf_v1::zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
	},
	wire::ObjectId,
};

#[derive(Dispatcher)]
pub struct DmabufFeedback;

impl ZwpLinuxDmabufFeedbackV1 for DmabufFeedback {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}
