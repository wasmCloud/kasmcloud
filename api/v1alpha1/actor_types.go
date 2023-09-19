/*
Copyright 2023.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

package v1alpha1

import (
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

// EDIT THIS FILE!  THIS IS SCAFFOLDING FOR YOU TO OWN!
// NOTE: json tags are required.  Any new fields you add must have json tags for the fields to be serialized.

// ActorSpec defines the desired state of Actor
type ActorSpec struct {
	Host     string `json:"host"`
	Image    string `json:"image"`
	Replicas uint   `json:"replicas"`
}

// ActorStatus defines the observed state of Actor
type ActorStatus struct {
	PublicKey       string `json:"publicKey"`
	DescriptiveName string `json:"descriptiveName,omitempty"`

	Caps               []string `json:"caps,omitempty"`
	CapabilityProvider []string `json:"capabilityProvider,omitempty"`
	CallAlias          *string  `json:"callAlias,omitempty"`
	Claims             Claims   `json:"claims"`

	Version   *string `json:"version,omitempty"`
	Reversion *int    `json:"reversion,omitempty"`

	Conditions        []metav1.Condition `json:"conditions"`
	Replicas          uint               `json:"replicas"`
	AvailableReplicas uint               `json:"availableReplicas"`
}

// +kubebuilder:object:root=true
// +kubebuilder:resource:path=actors,scope=Namespaced,categories=kasmcloud
// +kubebuilder:subresource:status
// +kubebuilder:subresource:scale:specpath=.spec.replicas,statuspath=.status.replicas
// +kubebuilder:printcolumn:name="Desc",type="string",JSONPath=".status.descriptiveName"
// +kubebuilder:printcolumn:name="PublicKey",type="string",JSONPath=".status.publicKey"
// +kubebuilder:printcolumn:name="Replicas",type="integer",JSONPath=".spec.replicas"
// +kubebuilder:printcolumn:name="AvailableReplicas",type="integer",JSONPath=".status.availableReplicas"
// +kubebuilder:printcolumn:name="Caps",type="string",JSONPath=".status.capabilityProvider"
// +kubebuilder:printcolumn:name="Image",type="string",JSONPath=".spec.image"

// Actor is the Schema for the actors API
type Actor struct {
	metav1.TypeMeta   `json:",inline"`
	metav1.ObjectMeta `json:"metadata,omitempty"`

	Spec   ActorSpec   `json:"spec,omitempty"`
	Status ActorStatus `json:"status,omitempty"`
}

//+kubebuilder:object:root=true

// ActorList contains a list of Actor
type ActorList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitempty"`
	Items           []Actor `json:"items"`
}

func init() {
	SchemeBuilder.Register(&Actor{}, &ActorList{})
}
